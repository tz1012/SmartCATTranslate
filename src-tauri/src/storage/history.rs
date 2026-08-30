use crate::storage::{CryptoBox, CryptoError, StorageDatabase};
use chrono::{Duration, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewHistoryRecord {
    pub kind: String,
    pub source_language: Option<String>,
    pub target_language: String,
    pub source: String,
    pub result: String,
    pub display_name: Option<String>,
    #[serde(default)]
    pub warning_count: u32,
    #[serde(default)]
    pub secret: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRecord {
    pub id: String,
    pub created_at: String,
    pub kind: String,
    pub source_language: Option<String>,
    pub target_language: String,
    pub source: String,
    pub result: String,
    pub display_name: Option<String>,
    pub warning_count: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub records: Vec<HistoryRecord>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPolicy {
    pub enabled: bool,
    pub retention_days: u16,
}

impl Default for HistoryPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: 30,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("history database unavailable")]
    Database(#[from] rusqlite::Error),
    #[error("history payload authentication failed")]
    Crypto(#[from] CryptoError),
    #[error("history record is invalid")]
    Invalid,
}

#[derive(Clone)]
pub struct HistoryStore {
    database: StorageDatabase,
    crypto: Arc<CryptoBox>,
}

impl HistoryStore {
    pub fn new(database: StorageDatabase, crypto: Arc<CryptoBox>) -> Self {
        Self { database, crypto }
    }

    pub fn save(&self, record: NewHistoryRecord) -> Result<Option<String>, HistoryError> {
        if record.secret {
            return Ok(None);
        }
        if record.kind.len() > 32 || record.target_language.len() > 64 {
            return Err(HistoryError::Invalid);
        }
        let id = Uuid::new_v4().simple().to_string();
        let source = self.crypto.seal_json(&record.source, &aad(&id, "source"))?;
        let result = self.crypto.seal_json(&record.result, &aad(&id, "result"))?;
        let display_name = record
            .display_name
            .as_ref()
            .map(|value| self.crypto.seal_json(value, &aad(&id, "display_name")))
            .transpose()?;
        let created_at = Utc::now().to_rfc3339();
        self.database
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .execute(
                "INSERT INTO history (id,created_at,kind,source_language,target_language,source_blob,result_blob,display_name_blob,warning_count) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![id, created_at, record.kind, record.source_language, record.target_language, source, result, display_name, record.warning_count],
            )?;
        Ok(Some(id))
    }

    pub fn list(&self, limit: u32, cursor: Option<&str>) -> Result<HistoryPage, HistoryError> {
        let limit = limit.clamp(1, 100) as usize;
        let connection = self.database.0.lock().unwrap_or_else(|p| p.into_inner());
        let mut statement = connection.prepare(
            "SELECT id,created_at,kind,source_language,target_language,source_blob,result_blob,display_name_blob,warning_count FROM history WHERE (?1 IS NULL OR created_at < ?1) ORDER BY created_at DESC,id DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![cursor, (limit + 1) as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Vec<u8>>(5)?,
                row.get::<_, Vec<u8>>(6)?,
                row.get::<_, Option<Vec<u8>>>(7)?,
                row.get::<_, u32>(8)?,
            ))
        })?;
        let mut values = Vec::new();
        for row in rows {
            values.push(row?);
        }
        let has_more = values.len() > limit;
        values.truncate(limit);
        let next_cursor = has_more
            .then(|| values.last().map(|value| value.1.clone()))
            .flatten();
        let records = values
            .into_iter()
            .map(
                |(
                    id,
                    created_at,
                    kind,
                    source_language,
                    target_language,
                    source,
                    result,
                    display_name,
                    warning_count,
                )| {
                    Ok(HistoryRecord {
                        source: self.crypto.open_json(&source, &aad(&id, "source"))?,
                        result: self.crypto.open_json(&result, &aad(&id, "result"))?,
                        display_name: display_name
                            .map(|value| self.crypto.open_json(&value, &aad(&id, "display_name")))
                            .transpose()?,
                        id,
                        created_at,
                        kind,
                        source_language,
                        target_language,
                        warning_count,
                    })
                },
            )
            .collect::<Result<Vec<_>, HistoryError>>()?;
        Ok(HistoryPage {
            records,
            next_cursor,
        })
    }

    pub fn read(&self, id: &str) -> Result<Option<HistoryRecord>, HistoryError> {
        if !valid_id(id) {
            return Err(HistoryError::Invalid);
        }
        let connection = self.database.0.lock().unwrap_or_else(|p| p.into_inner());
        let row = connection.query_row(
            "SELECT created_at,kind,source_language,target_language,source_blob,result_blob,display_name_blob,warning_count FROM history WHERE id=?1", [id],
            |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, Option<String>>(2)?,row.get::<_, String>(3)?,row.get::<_, Vec<u8>>(4)?,row.get::<_, Vec<u8>>(5)?,row.get::<_, Option<Vec<u8>>>(6)?,row.get::<_, u32>(7)?)),
        ).optional()?;
        row.map(
            |(
                created_at,
                kind,
                source_language,
                target_language,
                source,
                result,
                display_name,
                warning_count,
            )| {
                Ok(HistoryRecord {
                    id: id.to_owned(),
                    created_at,
                    kind,
                    source_language,
                    target_language,
                    source: self.crypto.open_json(&source, &aad(id, "source"))?,
                    result: self.crypto.open_json(&result, &aad(id, "result"))?,
                    display_name: display_name
                        .map(|value| self.crypto.open_json(&value, &aad(id, "display_name")))
                        .transpose()?,
                    warning_count,
                })
            },
        )
        .transpose()
    }

    pub fn delete(&self, id: &str) -> Result<bool, HistoryError> {
        if !valid_id(id) {
            return Err(HistoryError::Invalid);
        }
        Ok(self
            .database
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .execute("DELETE FROM history WHERE id=?1", [id])?
            > 0)
    }

    pub fn delete_all(&self) -> Result<u64, HistoryError> {
        Ok(self
            .database
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .execute("DELETE FROM history", [])? as u64)
    }

    pub fn purge_expired(&self, retention_days: u16) -> Result<u64, HistoryError> {
        let cutoff =
            (Utc::now() - Duration::days(i64::from(retention_days.clamp(1, 365)))).to_rfc3339();
        Ok(self
            .database
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .execute("DELETE FROM history WHERE created_at < ?1", [cutoff])? as u64)
    }
}

fn aad(id: &str, column: &str) -> Vec<u8> {
    format!("history:{id}:{column}").into_bytes()
}
fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|value| value.is_ascii_hexdigit())
}
