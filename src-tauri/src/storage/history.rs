use crate::{
    core::diagnostics::{DiagnosticEvent, DiagnosticEventName, DiagnosticOutcome},
    storage::{CryptoBox, CryptoError, StorageDatabase},
};
use chrono::{Duration, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicU16, Ordering},
    Arc,
};
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
    pub unreadable_count: u32,
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
    retention_days: Arc<AtomicU16>,
}

impl HistoryStore {
    pub fn new(database: StorageDatabase, crypto: Arc<CryptoBox>) -> Self {
        Self {
            database,
            crypto,
            // Zero means the persisted setting has not been loaded yet. Until it is,
            // save/list are deliberately non-destructive.
            retention_days: Arc::new(AtomicU16::new(0)),
        }
    }

    pub fn configure_retention(&self, days: u16) -> Result<(), HistoryError> {
        if !(1..=365).contains(&days) {
            DiagnosticEvent::new(
                DiagnosticEventName::HistoryMaintenance,
                DiagnosticOutcome::Failed,
            )
            .with_error_code("history_retention_invalid")
            .emit();
            return Err(HistoryError::Invalid);
        }
        self.retention_days.store(days, Ordering::Release);
        Ok(())
    }

    pub fn retention_configured(&self) -> bool {
        self.retention_days.load(Ordering::Acquire) != 0
    }

    pub fn save(&self, record: NewHistoryRecord) -> Result<Option<String>, HistoryError> {
        if record.secret {
            return Ok(None);
        }
        self.purge_current()?;
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
        DiagnosticEvent::new(
            DiagnosticEventName::HistoryMaintenance,
            DiagnosticOutcome::Succeeded,
        )
        .with_counts(1, 0)
        .emit();
        Ok(Some(id))
    }

    pub fn list(&self, limit: u32, cursor: Option<&str>) -> Result<HistoryPage, HistoryError> {
        self.purge_current()?;
        let limit = limit.clamp(1, 100) as usize;
        let cursor = cursor.map(parse_cursor).transpose()?;
        let connection = self.database.0.lock().unwrap_or_else(|p| p.into_inner());
        let mut statement = connection.prepare(
            "SELECT id,created_at,kind,source_language,target_language,source_blob,result_blob,display_name_blob,warning_count FROM history WHERE (?1 IS NULL OR created_at < ?1 OR (created_at = ?1 AND id < ?2)) ORDER BY created_at DESC,id DESC LIMIT ?3",
        )?;
        let cursor_created_at = cursor.as_ref().map(|value| value.0.as_str());
        let cursor_id = cursor.as_ref().map(|value| value.1.as_str());
        let rows = statement.query_map(
            params![cursor_created_at, cursor_id, (limit + 1) as i64],
            |row| {
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
            },
        )?;
        let mut values = Vec::new();
        for row in rows {
            values.push(row?);
        }
        let has_more = values.len() > limit;
        values.truncate(limit);
        let next_cursor = has_more
            .then(|| values.last().map(|value| format_cursor(&value.1, &value.0)))
            .flatten();
        let mut records = Vec::with_capacity(values.len());
        let mut unreadable_count = 0;
        for (
            id,
            created_at,
            kind,
            source_language,
            target_language,
            source,
            result,
            display_name,
            warning_count,
        ) in values
        {
            let decrypted = (|| {
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
            })();
            match decrypted {
                Ok(record) => records.push(record),
                Err(HistoryError::Crypto(_)) => unreadable_count += 1,
                Err(error) => return Err(error),
            }
        }
        Ok(HistoryPage {
            records,
            next_cursor,
            unreadable_count,
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
        self.configure_retention(retention_days)?;
        let cutoff = (Utc::now() - Duration::days(i64::from(retention_days))).to_rfc3339();
        let removed =
            self.database
                .0
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .execute("DELETE FROM history WHERE created_at < ?1", [cutoff])? as u64;
        DiagnosticEvent::new(
            DiagnosticEventName::HistoryMaintenance,
            DiagnosticOutcome::Succeeded,
        )
        .with_counts(removed, 0)
        .emit();
        Ok(removed)
    }

    fn purge_current(&self) -> Result<u64, HistoryError> {
        let days = self.retention_days.load(Ordering::Acquire);
        if days == 0 {
            DiagnosticEvent::new(
                DiagnosticEventName::HistoryMaintenance,
                DiagnosticOutcome::Failed,
            )
            .with_error_code("history_retention_pending")
            .emit();
            return Ok(0);
        }
        self.purge_expired(days)
    }
}

fn aad(id: &str, column: &str) -> Vec<u8> {
    format!("history:{id}:{column}").into_bytes()
}
fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|value| value.is_ascii_hexdigit())
}
fn format_cursor(created_at: &str, id: &str) -> String {
    format!("{created_at}|{id}")
}
fn parse_cursor(value: &str) -> Result<(String, String), HistoryError> {
    let (created_at, id) = value.rsplit_once('|').ok_or(HistoryError::Invalid)?;
    if created_at.is_empty()
        || !valid_id(id)
        || chrono::DateTime::parse_from_rfc3339(created_at).is_err()
    {
        return Err(HistoryError::Invalid);
    }
    Ok((created_at.to_owned(), id.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use zeroize::Zeroizing;

    fn record(source: &str) -> NewHistoryRecord {
        NewHistoryRecord {
            kind: "text".to_owned(),
            source_language: Some("en".to_owned()),
            target_language: "ko".to_owned(),
            source: source.to_owned(),
            result: format!("translated {source}"),
            display_name: None,
            warning_count: 0,
            secret: false,
        }
    }

    #[test]
    fn list_keeps_readable_records_when_one_encrypted_payload_is_damaged() {
        let root = tempdir().unwrap();
        let database = StorageDatabase::open(&root.path().join("history.sqlite3")).unwrap();
        let crypto = Arc::new(CryptoBox::from_zeroizing(Zeroizing::new([7_u8; 32])));
        let store = HistoryStore::new(database.clone(), crypto);
        store.configure_retention(30).unwrap();
        let damaged_id = store.save(record("damaged")).unwrap().unwrap();
        let readable_id = store.save(record("readable")).unwrap().unwrap();
        database
            .0
            .lock()
            .unwrap()
            .execute(
                "UPDATE history SET source_blob=?1 WHERE id=?2",
                params![b"invalid envelope".as_slice(), damaged_id],
            )
            .unwrap();

        let page = store
            .list(50, None)
            .expect("one damaged record must not hide readable history");

        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].id, readable_id);
        assert_eq!(page.unreadable_count, 1);
    }
}
