ALTER TABLE users ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1;
ALTER TABLE users ADD COLUMN disabled_at TEXT;

CREATE INDEX IF NOT EXISTS idx_users_role_enabled ON users(role, enabled);
