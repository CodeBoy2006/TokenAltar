PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS users (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  email TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  role TEXT NOT NULL CHECK (role IN ('admin', 'user')),
  display_name TEXT NOT NULL,
  points_balance REAL NOT NULL DEFAULT 0,
  anonymous_leaderboard INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS api_keys (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  key_prefix TEXT NOT NULL,
  key_hash TEXT NOT NULL UNIQUE,
  enabled INTEGER NOT NULL DEFAULT 1,
  spend_limit_points REAL,
  spent_points REAL NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS sessions (
  token_hash TEXT PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS channels (
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

CREATE TABLE IF NOT EXISTS channel_limits (
  channel_id INTEGER PRIMARY KEY REFERENCES channels(id) ON DELETE CASCADE,
  cycle_limit_tokens INTEGER NOT NULL,
  cycle_reset_day INTEGER NOT NULL CHECK (cycle_reset_day BETWEEN 1 AND 28),
  daily_limit_tokens INTEGER NOT NULL,
  hourly_limit_tokens INTEGER NOT NULL,
  used_cycle_tokens INTEGER NOT NULL DEFAULT 0,
  used_day_tokens INTEGER NOT NULL DEFAULT 0,
  used_hour_tokens INTEGER NOT NULL DEFAULT 0,
  fire_sale_days_before INTEGER NOT NULL DEFAULT 3,
  fire_sale_remaining_pct REAL NOT NULL DEFAULT 0.25,
  fire_sale_discount REAL NOT NULL DEFAULT 0.2,
  provider_share REAL NOT NULL DEFAULT 0.7,
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS model_prices (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  model_pattern TEXT NOT NULL UNIQUE,
  input_price_per_1k REAL NOT NULL,
  output_price_per_1k REAL NOT NULL,
  cache_price_per_1k REAL NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS affinity_rules (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL UNIQUE,
  enabled INTEGER NOT NULL DEFAULT 1,
  model_regex TEXT,
  request_path TEXT,
  user_agent_regex TEXT,
  key_source_type TEXT NOT NULL CHECK (key_source_type IN ('json_path', 'request_header', 'context')),
  key_source_path TEXT NOT NULL,
  group_name TEXT NOT NULL DEFAULT 'default',
  ttl_seconds INTEGER NOT NULL DEFAULT 3600,
  skip_retry_on_failure INTEGER NOT NULL DEFAULT 0,
  switch_on_success INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS affinity_bindings (
  cache_key TEXT PRIMARY KEY,
  rule_id INTEGER NOT NULL REFERENCES affinity_rules(id) ON DELETE CASCADE,
  channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  expires_at TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS ledger_entries (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  request_id TEXT NOT NULL UNIQUE,
  user_id INTEGER NOT NULL REFERENCES users(id),
  api_key_id INTEGER NOT NULL REFERENCES api_keys(id),
  channel_id INTEGER NOT NULL REFERENCES channels(id),
  provider_user_id INTEGER NOT NULL REFERENCES users(id),
  model TEXT NOT NULL,
  tokenizer TEXT NOT NULL,
  input_tokens INTEGER NOT NULL,
  output_tokens INTEGER NOT NULL,
  cache_tokens INTEGER NOT NULL,
  input_price_per_1k REAL NOT NULL,
  output_price_per_1k REAL NOT NULL,
  cache_price_per_1k REAL NOT NULL,
  surge_multiplier REAL NOT NULL,
  fire_sale_discount REAL NOT NULL,
  total_points REAL NOT NULL,
  provider_points REAL NOT NULL,
  status TEXT NOT NULL,
  formula_note TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS transfers (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  from_user_id INTEGER NOT NULL REFERENCES users(id),
  to_user_id INTEGER NOT NULL REFERENCES users(id),
  points REAL NOT NULL,
  memo TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS red_packets (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  creator_user_id INTEGER NOT NULL REFERENCES users(id),
  phrase TEXT NOT NULL UNIQUE,
  total_points REAL NOT NULL,
  remaining_points REAL NOT NULL,
  total_parts INTEGER NOT NULL,
  claimed_parts INTEGER NOT NULL DEFAULT 0,
  mode TEXT NOT NULL CHECK (mode IN ('even', 'lucky')),
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS red_packet_claims (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  red_packet_id INTEGER NOT NULL REFERENCES red_packets(id) ON DELETE CASCADE,
  user_id INTEGER NOT NULL REFERENCES users(id),
  points REAL NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE(red_packet_id, user_id)
);

CREATE TABLE IF NOT EXISTS system_settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT OR IGNORE INTO system_settings(key, value) VALUES
  ('invite_required', 'false'),
  ('surge_low_threshold', '0.30'),
  ('surge_high_threshold', '0.80');

INSERT OR IGNORE INTO model_prices(model_pattern, input_price_per_1k, output_price_per_1k, cache_price_per_1k) VALUES
  ('default', 1.0, 3.0, 0.2);
