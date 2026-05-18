PRAGMA foreign_keys = OFF;

CREATE TABLE IF NOT EXISTS channels_new (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  owner_user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  provider TEXT NOT NULL CHECK (provider IN ('openai', 'anthropic', 'gemini')),
  base_url TEXT NOT NULL,
  api_key_secret TEXT NOT NULL,
  models_json TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  status TEXT NOT NULL DEFAULT 'healthy',
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO channels_new(
  id, owner_user_id, name, provider, base_url, api_key_secret, models_json,
  enabled, status, created_at, updated_at
)
SELECT
  id, owner_user_id, name, provider, base_url, api_key_secret, models_json,
  enabled, status, created_at, updated_at
FROM channels;

DROP TABLE channels;

ALTER TABLE channels_new RENAME TO channels;

PRAGMA foreign_keys = ON;
