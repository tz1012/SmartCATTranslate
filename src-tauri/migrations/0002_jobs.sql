CREATE TABLE IF NOT EXISTS recovery_jobs (
  id TEXT PRIMARY KEY NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  kind TEXT NOT NULL,
  stage TEXT NOT NULL,
  completed INTEGER NOT NULL,
  total INTEGER NOT NULL,
  source_fingerprint TEXT NOT NULL,
  option_hash TEXT NOT NULL,
  display_name_blob BLOB NOT NULL,
  metadata_blob BLOB NOT NULL,
  payload_blob BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS recovery_jobs_expiry ON recovery_jobs(expires_at);
