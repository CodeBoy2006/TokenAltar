# TokenAltar

TokenAltar is a single-process Rust + SQLite gateway for pooling small-circle LLM API capacity.
It serves an operational Vue console and OpenAI/Anthropic/Gemini-compatible gateway endpoints from one binary.

## MVP Features

- `POST /v1/chat/completions`, `POST /v1/responses`, `POST /v1/messages`, and Gemini `POST /v1beta/models/{model}:generateContent` text routes with local API-key authentication.
- OpenAI Chat Completions, OpenAI Responses, Anthropic Messages, and Gemini Generate Content text adapters through a small shared text format.
- Same-protocol requests are passed through unchanged to matching upstream channel types; cross-protocol requests are converted only when needed.
- Local tiktoken precheck for OpenAI models, with deterministic Anthropic/Gemini proxy estimates and final settlement from upstream `usage`.
- SQLite WAL persistence for users, API keys, owner-scoped channels, global and channel-specific pricing, affinity rules, bindings, social economy, and ledger entries.
- In-memory routing state for cooldowns, surge metrics, and LRU affinity cache.
- MPSC ledger queue so gateway requests avoid synchronous high-frequency accounting writes.
- Vue console for login/register, API keys, channels, model prices, affinity rules, dashboard, ledger, settings, transfers, red packets, and leaderboards.
- Built Vue console assets are embedded into the Rust binary, so runtime deployment does not need a `frontend/dist` directory.

## Run

```bash
pnpm --dir frontend install
pnpm --dir frontend build
TOKENALTAR_ADMIN_EMAIL=admin@example.com \
TOKENALTAR_ADMIN_PASSWORD='change-me-now' \
cargo run
```

The server listens on `127.0.0.1:8080` by default and stores data in `tokenaltar.sqlite3`.
Run `pnpm --dir frontend build` before compiling Rust so the latest console assets are embedded into the binary.

## Environment

- `TOKENALTAR_BIND`: bind address, default `127.0.0.1:8080`.
- `TOKENALTAR_DATABASE_URL`: SQLite URL, default `sqlite://tokenaltar.sqlite3`.
- `TOKENALTAR_ADMIN_EMAIL` and `TOKENALTAR_ADMIN_PASSWORD`: create the first admin if missing.
- `TOKENALTAR_LEADERBOARD_TIMEZONE`: optional IANA timezone for day/month leaderboard windows, for example `Asia/Shanghai`; defaults to the server local timezone.

## Gateway Notes

Client requests must use `Authorization: Bearer sk-...`.
Console sessions use `Authorization: Bearer ta-...`.

Text protocol conversion supports text messages, image inputs, `system`, `temperature`, max token controls, and basic tool/function fields across OpenAI, Anthropic, and Gemini.
Files, embeddings, rerank, realtime, audio, and other non-text extensions are intentionally outside the current gateway surface.
Before any upstream bytes are sent to the client, retryable upstream failures (`408`, `429`, and `5xx`) and connection errors release the local reservation, put that channel into a short local cooldown, and transparently retry another healthy channel when the matching affinity rule permits fallback.
When `switch_on_success` is enabled, a successful fallback rewrites the affinity binding to the recovered channel; `skip_retry_on_failure` keeps a bound request pinned and returns the bound channel error instead.
Streaming requests follow the same pre-stream retry rule, but once a successful upstream stream has begun the gateway does not interrupt or replay it.

## Management Controls

API keys are managed from the console and through `/api/api-keys`.
Each key supports enable/disable, soft deletion, one-time rotation, optional expiration, a cumulative point spend limit, and an optional model allow-list.
Model allow-lists accept exact model names and prefix wildcards such as `gpt-4o*`; empty allow-lists allow every model.
Deleted keys are hidden and unusable, but ledger history remains intact.

Channels are managed from `/api/channels`.
Owners can edit provider, URL, model coverage, arbitrary quota windows, fire-sale thresholds, provider share, and enabled state.
Each quota window defines a token limit, period unit/count, anchor timestamp, and IANA timezone; every configured window is enforced.
When editing an existing channel, an empty `api_key_secret` keeps the stored upstream secret.
The console also exposes channel clone, health test, per-row enable/disable, batch enable/disable, and soft delete operations.
Health tests record `health_checked_at`, `upstream_latency_ms`, and `last_error` for operator visibility; they do not automatically disable routing.

Runtime settings are managed from `/api/settings`, with the current typed view available at `/api/runtime-settings`.
Admins can configure seed balances, pricing units and fallback prices, settlement rounding, surge thresholds and multipliers, routing retry/cooldown/weight knobs, ledger/cache capacities, and console defaults for new keys and channels.
Settings are stored in `system_settings`; request-time economy and routing values are read from the database so most changes apply without rebuilding.
Startup-sized values such as ledger queue capacity and affinity cache capacity apply when the process starts.

## Operational Notes

- Channel token windows are refreshed on startup, dashboard/channel reads, and gateway requests.
- Channel status moves to `cooling` when any configured quota window is exhausted.
- Regular users can add and manage their own upstream channels. Console channel reads are owner-scoped for regular users and always redact upstream API keys.
- Model prices are matched per channel first, then fall back to global model defaults managed by admins.
- If no model price row matches, fallback input/output/cache rates come from Settings instead of code constants.
- Invite-gated registration is controlled by `invite_required` and `invite_code_default` in the Settings tab.
- Red packet claims are transaction guarded with unique `(packet, user)` claims.
- Leaderboards support `period=day` and `period=month`, count successful ledger entries only, and mask users that enable anonymous ranking.

## Verify

```bash
cargo test
cargo clippy -- -D warnings
pnpm --dir frontend build
cargo build --release
```
