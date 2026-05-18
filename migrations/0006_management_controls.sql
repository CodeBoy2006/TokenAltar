ALTER TABLE api_keys ADD COLUMN expires_at TEXT;
ALTER TABLE api_keys ADD COLUMN allowed_models_json TEXT;
ALTER TABLE api_keys ADD COLUMN last_used_at TEXT;
ALTER TABLE api_keys ADD COLUMN updated_at TEXT;
ALTER TABLE api_keys ADD COLUMN deleted_at TEXT;

UPDATE api_keys
SET allowed_models_json = '[]',
    updated_at = COALESCE(created_at, datetime('now'))
WHERE allowed_models_json IS NULL;

ALTER TABLE channels ADD COLUMN deleted_at TEXT;
ALTER TABLE channels ADD COLUMN health_checked_at TEXT;
ALTER TABLE channels ADD COLUMN upstream_latency_ms INTEGER;
ALTER TABLE channels ADD COLUMN last_error TEXT;
