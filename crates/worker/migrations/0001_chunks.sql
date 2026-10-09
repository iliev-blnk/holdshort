-- One row per 5-min live chunk part; key = raw/live/YYYY-MM-DD/HHMM[-n].ndjson
CREATE TABLE IF NOT EXISTS chunks (
  key TEXT PRIMARY KEY,
  body TEXT NOT NULL,
  created_ms INTEGER NOT NULL
);
