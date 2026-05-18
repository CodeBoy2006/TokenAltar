PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS model_prices_new (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  channel_id INTEGER REFERENCES channels(id) ON DELETE CASCADE,
  model_pattern TEXT NOT NULL,
  input_price_per_1k REAL NOT NULL,
  output_price_per_1k REAL NOT NULL,
  cache_price_per_1k REAL NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE(channel_id, model_pattern)
);

INSERT INTO model_prices_new(
  id, channel_id, model_pattern, input_price_per_1k, output_price_per_1k, cache_price_per_1k, created_at
)
SELECT id, NULL, model_pattern, input_price_per_1k, output_price_per_1k, cache_price_per_1k, created_at
FROM model_prices;

DROP TABLE model_prices;

ALTER TABLE model_prices_new RENAME TO model_prices;

CREATE UNIQUE INDEX IF NOT EXISTS idx_model_prices_global_pattern
  ON model_prices(model_pattern)
  WHERE channel_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_model_prices_channel
  ON model_prices(channel_id, model_pattern);
