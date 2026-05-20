CREATE TABLE IF NOT EXISTS api_key_channels (
  api_key_id INTEGER NOT NULL REFERENCES api_keys(id) ON DELETE CASCADE,
  channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY(api_key_id, channel_id)
);

INSERT OR IGNORE INTO api_key_channels(api_key_id, channel_id)
SELECT k.id, c.id
FROM api_keys k
JOIN channels c ON 1 = 1
WHERE k.deleted_at IS NULL
  AND c.deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_api_key_channels_channel
  ON api_key_channels(channel_id);
