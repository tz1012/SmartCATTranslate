use rusqlite::Connection;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

const HISTORY_MIGRATION: &str = include_str!("../../migrations/0001_history.sql");
const JOBS_MIGRATION: &str = include_str!("../../migrations/0002_jobs.sql");

#[derive(Clone)]
pub struct StorageDatabase(pub(crate) Arc<Mutex<Connection>>);

impl StorageDatabase {
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|_| rusqlite::Error::InvalidPath(path.into()))?;
        }
        let mut connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "secure_delete", "ON")?;
        let transaction = connection.transaction()?;
        let version: u32 =
            transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version < 1 {
            transaction.execute_batch(HISTORY_MIGRATION)?;
            transaction.pragma_update(None, "user_version", 1_u32)?;
        }
        if version < 2 {
            transaction.execute_batch(JOBS_MIGRATION)?;
            transaction.pragma_update(None, "user_version", 2_u32)?;
        }
        transaction.commit()?;
        Ok(Self(Arc::new(Mutex::new(connection))))
    }
}
