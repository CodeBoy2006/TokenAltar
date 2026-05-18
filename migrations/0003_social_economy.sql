CREATE INDEX IF NOT EXISTS idx_ledger_provider_month ON ledger_entries(provider_user_id, created_at);
CREATE INDEX IF NOT EXISTS idx_ledger_consumer_month ON ledger_entries(user_id, created_at);
CREATE INDEX IF NOT EXISTS idx_transfers_users ON transfers(from_user_id, to_user_id, created_at);
CREATE INDEX IF NOT EXISTS idx_red_packet_phrase ON red_packets(phrase);
