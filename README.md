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
- Vue console for login/register, user management, API keys, channels, model prices, affinity rules, dashboard, ledger, settings, transfers, red packets, leaderboards, and the visual project guide.
- Authenticated SSE console updates through `/api/events`, so ledger settlement, channel health, settings, and account/economy mutations refresh the affected console panels without full-page polling.
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
The live console stream is `GET /api/events` with the same console bearer token and `Accept: text/event-stream`; events contain resource-topic invalidations, and clients should reload the existing REST endpoints for filtered data.

Text protocol conversion supports text messages, image inputs, `system`, `temperature`, max token controls, and basic tool/function fields across OpenAI, Anthropic, and Gemini.
Files, embeddings, rerank, realtime, audio, and other non-text extensions are intentionally outside the current gateway surface.
Before any semantic upstream content is sent to the client, retryable upstream failures (`408`, `429`, and `5xx`), connection errors, and semantic-empty replies release the local reservation, put that channel into a short local cooldown, and transparently retry another healthy channel when the matching affinity rule permits fallback.
When `switch_on_success` is enabled, a successful fallback rewrites the affinity binding to the recovered channel; `skip_retry_on_failure` keeps a bound request pinned and returns the bound channel error instead.
For streaming requests, heartbeat/comment frames, usage metadata, whitespace-only deltas, and terminal markers do not count as semantic content; the gateway keeps the client stream unopened for those frames and can switch channels if the upstream stream ends empty. Once text or tool-call semantics have been forwarded, the gateway does not interrupt or replay that stream.

## Management Controls

Admins can manage accounts from `/api/users`.
User management supports account creation, profile/role/balance edits, enable/disable, and password resets.
Disabled users cannot log in or authenticate through existing sessions or API keys; disabling also turns off their active API keys and channels while preserving ledger history.
The console prevents removing the last enabled admin or disabling the current admin session.

API keys are managed from the console and through `/api/api-keys`.
Each key supports enable/disable, soft deletion, one-time rotation, optional expiration, a cumulative point spend limit, and an optional model allow-list.
Model allow-lists accept exact model names and prefix wildcards such as `gpt-4o*`; empty allow-lists allow every model.
Deleted keys are hidden and unusable, but ledger history remains intact.

Channels are managed from `/api/channels`.
Owners can edit provider, URL, model coverage, arbitrary quota windows, fire-sale thresholds, provider share, and enabled state.
Each quota window defines a token limit, period unit/count, anchor timestamp, and IANA timezone; every configured window is enforced.
When editing an existing channel, an empty `api_key_secret` keeps the stored upstream secret.
The console also exposes channel clone, health test, per-row enable/disable, batch enable/disable, and soft delete operations.
Channel health is passive by default: real gateway attempts append health events for `available`, `empty`, `degraded`, and `down` outcomes while manual health tests remain an explicit operator action.
The channel list returns 48 fixed 30-minute health windows; each window averages TTFT from successful non-empty events only, excludes failed or empty events from the TTFT average, and renders windows with no records as gray.
The console has a dedicated Health page that shows all visible channels, labels each provider, and summarizes request-derived samples, empty replies, down windows, and TTFT alongside the 48-window strip.

Runtime settings are managed from `/api/settings`, with the current typed view available at `/api/runtime-settings`.
Admins can configure seed balances, pricing units and fallback prices, settlement rounding, surge thresholds and multipliers, routing retry/cooldown/weight knobs, ledger/cache capacities, and console defaults for new keys and channels.
Settings are stored in `system_settings`; request-time economy and routing values are read from the database so most changes apply without rebuilding.
Startup-sized values such as ledger queue capacity and affinity cache capacity apply when the process starts.
Surge pressure compares tokens settled in the last hour with the current primary quota-window capacity converted to a tokens-per-hour rate; when no healthy primary-window capacity exists, the dashboard reports `no_capacity` without applying peak pricing.

## Operational Notes

- Channel token windows are refreshed on startup, dashboard/channel reads, and gateway requests.
- Channel status moves to `cooling` when any configured quota window is exhausted.
- Channel health history is inferred from actual request outcomes rather than scheduled probes. Empty semantic replies and upstream failures are retained as window status samples, but they do not contribute to average TTFT.
- Regular users can add and manage their own upstream channels. Console channel reads are owner-scoped for regular users and always redact upstream API keys.
- Model prices are matched per channel first, then fall back to global model defaults managed by admins.
- Console live updates are topic-based invalidations, not full data snapshots; REST endpoints remain the permission boundary for owner-scoped and admin-only data.
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
