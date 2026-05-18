# TokenAltar

TokenAltar is a single-process Rust + SQLite gateway for pooling small-circle LLM API capacity.
It serves an operational Vue console and OpenAI/Anthropic-compatible gateway endpoints from one binary.

## MVP Features

- `POST /v1/responses` and `POST /v1/messages` with local API-key authentication.
- OpenAI Responses and Anthropic Messages adapters through a shared internal chat format.
- SQLite WAL persistence for users, API keys, channels, pricing, affinity rules, bindings, and ledger entries.
- In-memory routing state for cooldowns, surge metrics, and LRU affinity cache.
- MPSC ledger queue so gateway requests avoid synchronous high-frequency accounting writes.
- Vue console for login, API keys, channels, model prices, affinity rules, dashboard, and ledger.

## Run

```bash
pnpm --dir frontend install
pnpm --dir frontend build
TOKENALTAR_ADMIN_EMAIL=admin@example.com \
TOKENALTAR_ADMIN_PASSWORD='change-me-now' \
cargo run
```

The server listens on `127.0.0.1:8080` by default and stores data in `tokenaltar.sqlite3`.

## Environment

- `TOKENALTAR_BIND`: bind address, default `127.0.0.1:8080`.
- `TOKENALTAR_DATABASE_URL`: SQLite URL, default `sqlite://tokenaltar.sqlite3`.
- `TOKENALTAR_FRONTEND_DIST`: built Vue directory, default `frontend/dist`.
- `TOKENALTAR_ADMIN_EMAIL` and `TOKENALTAR_ADMIN_PASSWORD`: create the first admin if missing.

## Gateway Notes

Client requests must use `Authorization: Bearer sk-...`.
Console sessions use `Authorization: Bearer ta-...`.

MVP protocol conversion supports text messages, `system`, `temperature`, max token controls, and basic tool/function fields.
Images, files, and reasoning/thinking extensions are rejected or left for same-protocol future work.
