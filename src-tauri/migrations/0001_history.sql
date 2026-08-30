CREATE TABLE IF NOT EXISTS history (
  id TEXT PRIMARY KEY NOT NULL,
  created_at TEXT NOT NULL,
  kind TEXT NOT NULL,
  source_language TEXT,
  target_language TEXT NOT NULL,
  source_blob BLOB NOT NULL,
  result_blob BLOB NOT NULL,
  display_name_blob BLOB,
  warning_count INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS history_created_at ON history(created_at DESC, id DESC);
