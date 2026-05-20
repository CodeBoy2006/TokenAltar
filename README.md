

<p align="center">
  <img src="assets/banner.png" alt="TokenAltar logo" width="400" />
</p>

> **TokenAltar is a self-contained LLM capacity console for small teams, research groups, private circles, and API-sharing communities.**


![TokenAltar project guide](frontend/public/guides/tokenaltar-project-guide.png)


It pools upstream OpenAI, Anthropic, Gemini, and compatible model capacity behind one local gateway. Members call TokenAltar with local API keys; providers contribute upstream channels; the console keeps routing, quotas, pricing, health, point settlement, and social-economy flows visible from one embedded Vue UI.

TokenAltar runs as a single Rust process with SQLite persistence and embedded frontend assets. There is no separate web server to deploy after the console is built.

## What It Does

TokenAltar turns scattered LLM accounts into a governed shared pool:

- **Unified gateway:** expose OpenAI Chat Completions, OpenAI Responses, Anthropic Messages, and Gemini Generate Content routes through local `sk-...` keys.
- **Provider channels:** let users or admins contribute upstream model channels with model coverage, quota windows, fire-sale policy, and globally governed provider-share settlement.
- **Scoped client keys:** issue keys with enable/disable, soft deletion, rotation, optional expiration, model allow-lists, spend limits, and explicit channel allow-lists.
- **Dynamic routing:** choose healthy channels by key scope, model coverage, quota state, cooldown, route weight, fire-sale state, and affinity binding.
- **Point economy:** settle usage into points, reward providers, support P2P transfers, phrase red packets, and day/month leaderboards.
- **Operator console:** manage users, API keys, channels, prices, affinity rules, ledger entries, runtime settings, health windows, and the visual Guide page from the same binary.

## Core Flow

```text
member account
  -> local API key
  -> TokenAltar gateway
  -> scoped, healthy upstream channel
  -> upstream model response
  -> usage settlement
  -> consumer debit + provider reward + ledger record
```

![Flow](assets/core-flow.png)

The gateway estimates input usage before forwarding a request, reserves local points and channel quota, then settles against upstream `usage` when the response completes. Failed retry attempts release their reservation before another eligible channel is tried.

## Console Pages

| Page | Purpose |
| --- | --- |
| Dashboard | Live capacity, surge state, available tokens, enabled channels, and current point spend. |
| Users | Admin-only account management, roles, balances, password resets, and account suspension. |
| API Keys | Client credentials, model fences, spend ceilings, channel allow-lists, rotation, and soft deletion. |
| Channels | Upstream providers, model coverage, quota windows, fire-sale economics, cloning, testing, and batch enable/disable. |
| Health | Passive request-derived health windows, TTFT, empty replies, degraded samples, and down windows. |
| Pricing | Per-1M-token model tariffs with channel overrides before global defaults. |
| Affinity | Admin-only sticky routing rules for tenant/session/cache locality. |
| Economy | Point transfers, phrase red packets, claim history, and anonymous leaderboard preference. |
| Leaderboards | Day/month provider-token and consumer-point rankings. |
| Ledger | Settlement archive with token counts, tokenizer notes, and formula text. |
| Guide | Visual product map plus a user-facing rulebook for access, capacity, routing, settlement, health, and economy. |
| Settings | Admin-only runtime controls for invitation, economy, routing, pricing fallback, and defaults. |

Regular users see owner-scoped channel and pricing views. Admin-only data remains behind backend permission checks; console live updates are topic invalidations, not privileged data snapshots.

## Gateway Compatibility

Client requests authenticate with `Authorization: Bearer sk-...`.

Supported gateway routes:

| Client route | Intended compatibility |
| --- | --- |
| `POST /v1/chat/completions` | OpenAI Chat Completions-style clients |
| `POST /v1/responses` | OpenAI Responses-style clients |
| `POST /v1/messages` | Anthropic Messages-style clients |
| `POST /v1beta/models/{model}:generateContent` | Gemini Generate Content-style clients |
| `POST /v1beta/models/{model}:streamGenerateContent` | Gemini streaming clients |

Text protocol conversion supports text messages, image inputs, `system`, `temperature`, max-token controls, and basic tool/function fields across OpenAI, Anthropic, and Gemini. Same-protocol traffic is passed through unchanged to matching upstream channel types; cross-protocol traffic is converted only when needed.

Files, embeddings, rerank, realtime, audio, and other non-text extensions are intentionally outside the current gateway surface.

## Routing And Reliability

Routing starts from the local API key. A request can only use channels allowed by that key, then must pass model coverage, enabled/status checks, quota-window checks, cooldown checks, and affinity policy.

Retryable upstream failures release local reservations and can fall back to another healthy channel before semantic content reaches the client. Retryable cases include connection errors, `408`, `429`, `5xx`, and semantic-empty replies. For streams, heartbeat/comment frames, usage-only metadata, whitespace-only deltas, and terminal markers do not count as semantic content; once real text or tool-call semantics have been forwarded, the gateway does not interrupt or replay that stream.

Built-in affinity presets cover:

- GPT Responses: `^gpt-.*$`, `/v1/responses`, `prompt_cache_key`
- Claude Messages: `^claude-.*$`, `/v1/messages`, `metadata.user_id`
- Gemini Generate Content: `^gemini-.*$`, generate/stream routes, `cachedContent`

These presets use a 3600-second TTL, enable successful fallback switching, and keep cache-local bound traffic pinned on bound-channel failure. They intentionally omit model name from the cache key, matching model-family locality behavior.

## Capacity, Pricing, And Settlement

Channels can define arbitrary quota windows. Each window has a token limit, period unit/count, anchor timestamp, and IANA timezone. Every configured window is enforced. If any configured window is exhausted, the channel stops routing until the relevant window refreshes.

The first quota window is the primary inventory window used for dashboard availability, route weighting, surge normalization, and fire-sale reset timing.

Prices are stored per **1M tokens** and split into input, output, and cache-token rates. Price resolution is:

1. channel-scoped model pattern
2. channel-scoped `default`
3. global model pattern
4. global `default`
5. runtime fallback rates from Settings

Settlement applies the selected tariff, surge multiplier, fire-sale discount, admin-configured global provider share, and configured rounding. Ledger entries keep the final point amount and a readable formula note.

Built-in global presets currently cover GPT-5.5, GPT-5.4, GPT-5.3-codex, GPT-5.2, GPT-5.2-codex, Claude Opus 4.7/4.6/4.5/4.1/4, Claude Sonnet 4.6/4.5/4, and Claude Haiku 4.5. The single cache price field represents cached-input/cache-hit pricing.

## Health Model

Channel health is passive by default. Real gateway attempts append health events for `available`, `empty`, `degraded`, and `down` outcomes. Manual health tests are still available from the console, but there is no scheduled background probing.

The Health page displays 48 fixed 30-minute windows over the most recent 24 hours. TTFT averages include successful non-empty samples only. Failed, degraded, and empty events affect status color/counts but do not contribute to TTFT averages. Windows with no records render as gray.

## Live Console Updates

Console sessions authenticate with `Authorization: Bearer ta-...`.

The live stream is:

```http
GET /api/events
Accept: text/event-stream
Authorization: Bearer ta-...
```

Events carry resource topics such as ledger, channels, settings, economy, health, users, prices, and leaderboards. The Vue console reloads the affected REST endpoints after receiving an invalidation.

## Run Locally

Prerequisites:

- Rust toolchain with `cargo`
- Node.js and `pnpm`
- SQLite is used through the Rust application; no external database service is required

Build the console and start the server:

```bash
pnpm --dir frontend install
pnpm --dir frontend build

TOKENALTAR_ADMIN_EMAIL=admin@example.com \
TOKENALTAR_ADMIN_PASSWORD='change-me-now' \
cargo run
```

The server listens on `127.0.0.1:8080` by default and stores data in `tokenaltar.sqlite3`.

Run `pnpm --dir frontend build` before compiling or releasing Rust whenever the console changes. Built assets are embedded into the Rust binary, so runtime deployment does not need a `frontend/dist` directory.

## Configuration

Environment variables:

| Variable | Default | Purpose |
| --- | --- | --- |
| `TOKENALTAR_BIND` | `127.0.0.1:8080` | HTTP bind address. |
| `TOKENALTAR_DATABASE_URL` | `sqlite://tokenaltar.sqlite3` | SQLite database URL. |
| `TOKENALTAR_ADMIN_EMAIL` | unset | Creates the first admin if no admin exists. |
| `TOKENALTAR_ADMIN_PASSWORD` | unset | Initial password for the first admin. |
| `TOKENALTAR_LEADERBOARD_TIMEZONE` | server local timezone | Optional IANA timezone for day/month leaderboard windows, for example `Asia/Shanghai`. |

Runtime settings live in the console Settings page and `/api/runtime-settings`. Admins can configure invitation policy, seed balances, per-1M fallback prices, rounding, surge thresholds and multipliers, global provider share, retry/cooldown behavior, route weighting, queue/cache capacities, and defaults for new keys and channels.

Most request-time economy and routing settings apply without rebuilding. Startup-sized values, such as ledger queue capacity and affinity cache capacity, apply when the process starts.

## Operational Notes

- Channel token windows refresh on startup, dashboard/channel reads, and gateway requests.
- Disabled users cannot log in or authenticate through existing sessions or API keys.
- Disabling a user also disables their active API keys and channels while preserving ledger history.
- Deleted API keys and channels are hidden and unusable, but historical ledger rows remain.
- Regular users can add and manage their own upstream channels; upstream API secrets are always redacted from console responses.
- New API keys default to every current route channel. Keys that still cover the full route pool are automatically granted newly created channels; manually narrowed keys stay narrowed.
- Invite-gated registration is controlled by `invite_required` and `invite_code_default` in Settings.
- Red packet claims are transaction guarded with unique `(packet, user)` claims.
- Leaderboards support `period=day` and `period=month`, count successful ledger entries only, and mask users that enable anonymous ranking.

## Verify

```bash
cargo test
cargo clippy -- -D warnings
pnpm --dir frontend build
cargo build --release
```

For visual changes, run the app against a temporary SQLite database and inspect the relevant console pages with Playwright or a browser.
