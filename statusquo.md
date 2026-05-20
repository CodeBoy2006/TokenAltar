# Status Quo

## [2026-05-18 12:57] TokenAltar MVP Bootstrap
- **Changes:** Created the Rust Axum/SQLite backend, Vue/Vite console, SQLite migrations, OpenAI Responses and Anthropic Messages gateway adapters, routing/affinity/fire-sale logic, MPSC ledger worker, pricing engine, tests, README, and ignore rules.
- **Status:** Completed
- **Next Steps:** Configure real upstream channels in the console, run with production admin credentials, and add provider-specific streaming event normalization as usage grows.
- **Context:** MVP rejects multimodal and reasoning/thinking extensions; token precheck uses a conservative local estimator while final settlement uses upstream usage.

## [2026-05-18 13:27] Full PRD Completion Pass
- **Changes:** Added Chat Completions gateway support, tiktoken-based precheck, quota window refresh/status transitions, invite settings, P2P transfers, red packets, monthly leaderboards, anonymous ranking, and a complete Vue console for the new workflows.
- **Status:** Completed
- **Next Steps:** Configure production upstream channels and run an external-provider smoke test with real API keys.
- **Context:** Multimodal and reasoning/thinking payloads remain intentionally outside the text/tool MVP boundary; Anthropic local precheck uses the documented proxy estimator while settlement uses returned usage.

## [2026-05-18 14:14] Neoclassical Console Redesign
- **Changes:** Reworked the Vue console into a neoclassical control surface, added typed tab metadata and dashboard metric cards, replaced the global visual system with stone/gold/bronze accented responsive layouts, and added `frontend/public/altar-relief.svg` as a local decorative relief asset.
- **Status:** Completed
- **Next Steps:** Review with real production channel/ledger data to tune table density if rows become very wide.
- **Context:** Verified with `pnpm --dir frontend build` plus Playwright desktop/mobile login and dashboard/channel screenshots against a temporary local backend.

## [2026-05-18 14:28] Oil Painting Background Asset
- **Changes:** Moved the generated `image.png` into `frontend/public/tokenaltar-background.png` and wired it into the login hero, ambient shell artwork, and page header background treatments.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Rebuilt the frontend and checked login/dashboard desktop and mobile rendering with Playwright against a temporary local backend.

## [2026-05-18 14:53] Owner Channel Pricing
- **Changes:** Opened channel creation to regular users, redacted upstream API secrets from console channel responses, owner-scoped regular-user channel reads, added channel-specific model price overrides with global fallback, and updated the Vue pricing scope controls.
- **Status:** Completed
- **Next Steps:** Run a production smoke test with one regular-user channel and one admin global default price.
- **Context:** Gateway routing still uses full internal channel records, while console APIs return masked channel DTOs; channel prices override global model patterns only for the selected upstream channel.

## [2026-05-18 15:21] Text Protocol Compatibility Pass
- **Changes:** Reworked the gateway protocol layer into a text-only client/provider protocol matrix, added same-protocol passthrough for OpenAI Responses, Anthropic Messages, and Gemini Generate Content, added cross-protocol text conversion paths, introduced Gemini upstream channels and routes, and updated README/frontend provider selection.
- **Status:** Completed
- **Next Steps:** Add provider-specific golden fixtures as real upstream incompatibilities are found.
- **Context:** new-api was cloned to `/tmp/tokenaltar-new-api-reference` for architecture reference only; no third-party source was copied into this repository.

## [2026-05-18 15:31] Text Protocol Boundary Tightening
- **Changes:** Narrowed Gemini path handling so the route action only influences temporary parse state, kept affinity/body inspection on the original request payload, and added a regression test to verify Gemini same-protocol passthrough does not leak internal fields.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Text-only protocol support remains limited to chat/responses/messages/gemini generate content; embeddings, rerank, images, and realtime are still out of scope.

## [2026-05-18 15:58] Antikythera Hero Refinement
- **Changes:** Removed Platonic Solids and Janus hero overlays, kept only the animated Antikythera mechanism layer, and fixed the mid-width login hero overlap by adding container-query breakpoints, safer grid tracks, and an optional word-break point in the TokenAltar title.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Verified `pnpm --dir frontend build`; checked 1366x900 and 2048x1373 hero/title/card geometry for no overlap and zero horizontal page overflow.

## [2026-05-18 17:57] Daily Leaderboards
- **Changes:** Added day/month leaderboard periods, success-only leaderboard aggregation, configurable leaderboard window timezone via `TOKENALTAR_LEADERBOARD_TIMEZONE`, Vue day/month controls, README documentation, and regression coverage for daily filtering.
- **Status:** Completed
- **Next Steps:** Set `TOKENALTAR_LEADERBOARD_TIMEZONE` in production if the deployment should use a fixed IANA timezone such as `Asia/Shanghai`.
- **Context:** Defaults to the server local timezone when the environment variable is unset; leaderboard window starts are converted to SQLite UTC datetime strings before querying ledger rows.

## [2026-05-18 16:24] Image Input Protocol Support
- **Changes:** Extended the shared protocol layer to parse and serialize image parts for OpenAI Chat Completions, OpenAI Responses, Anthropic Messages, and Gemini Generate Content; added conservative image token prechecks; added Gemini image normalization in the gateway so external URLs are fetched and converted to `inlineData` before forwarding; updated tests and README gateway notes.
- **Status:** Completed
- **Next Steps:** Add provider-specific edge cases if a live upstream rejects any of the accepted image shapes.
- **Context:** Same-protocol passthrough remains direct; cross-protocol text plus image conversion is intentionally minimal and limited to the current text API surface.

## [2026-05-18 19:43] Management Controls Deepening
- **Changes:** Added API key update/rotate/batch-delete lifecycle controls, channel update/copy/test/batch-enable lifecycle controls, soft-delete visibility rules, gateway model allow-list enforcement, and a polished console for filtering, selecting, editing, testing, cloning, and retiring keys/channels.
- **Status:** Completed
- **Next Steps:** Smoke test one real upstream channel health check and one rotated client key in production credentials.
- **Context:** Channel and API key deletion is soft to preserve ledger history; empty channel API key updates keep the existing upstream secret.

## [2026-05-18 22:25] Fire Sale Reset Window Fix
- **Changes:** Corrected fire-sale activation so it requires both the remaining-token threshold and a real UTC distance-to-reset window before applying discounted routing/pricing.
- **Status:** Completed
- **Next Steps:** Confirm production billing reset timezone expectations if they should differ from the existing UTC channel-window logic.
- **Context:** Targeted pricing tests cover in-window, out-of-window, reset-day, and remaining-threshold behavior.

## [2026-05-18 22:32] Affinity Cache TTL Fix
- **Changes:** Stored affinity expiration timestamps inside the in-memory LRU cache and dropped expired cache hits before routing.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** SQLite affinity bindings remain the source of truth; the cache now mirrors DB TTL instead of treating a cached channel ID as permanent.

## [2026-05-18 22:34] Rolling Surge Window Fix
- **Changes:** Replaced the permanent surge token counter with a rolling one-hour in-memory token window using minute buckets.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Surge pricing now bases its load ratio on recent gateway settlements instead of all settlements since process start.

## [2026-05-18 22:41] Gateway Reservation Safety
- **Changes:** Added atomic gateway request reservations for estimated points and channel tokens, released reservations on upstream failure, settled only final deltas, guarded duplicate ledger side effects, and made P2P transfer debits conditional on available balance.
- **Status:** Completed
- **Next Steps:** Consider moving reservation state to a durable recovery table if the process must survive crashes between reserve and settlement.
- **Context:** Existing gateway integration tests pass; new tests cover reservation release and settlement delta accounting.

## [2026-05-18 23:23] Arbitrary Channel Quota Windows
- **Changes:** Replaced fixed cycle/day/hour quota handling with arbitrary per-channel quota windows, added normalized SQLite window storage, switched reservation/settlement/routing/surge/fire-sale logic to the unified window path, and updated the Vue channel console to create and display any number of windows.
- **Status:** Completed
- **Next Steps:** Configure production channels with the exact upstream billing windows and timezones before routing live traffic.
- **Context:** Each window is a hard quota constraint; the first window is the primary inventory window used for dashboard availability, routing weight, and fire-sale reset timing.

## [2026-05-18 23:29] Quota Window Count Contract
- **Changes:** Removed the artificial per-channel quota window count cap so the API accepts any positive number of windows while still requiring each window to have valid period and anchor semantics.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Runtime cost scales linearly with configured windows because every gateway reservation checks all channel windows atomically.

## [2026-05-18 23:41] Role Aware Console UI
- **Changes:** Hid admin-only Affinity navigation from regular users, adjusted user-facing channel/pricing copy, guarded regular-user price saves without a channel, and added owner labels to admin channel/pricing tables.
- **Status:** Completed
- **Next Steps:** Add action-level error toasts and confirmations for destructive channel/key operations.
- **Context:** Backend permissions are unchanged; this is a UI clarity pass over existing role boundaries.

## [2026-05-19 12:47] Gateway Failure Retry
- **Changes:** Reworked gateway upstream dispatch into a bounded retry loop for connection errors, 408, 429, and 5xx responses; failed attempts now release reservations, apply a short channel cooldown, retry another healthy channel when affinity fallback is allowed, and update affinity bindings on successful fallback.
- **Status:** Completed
- **Next Steps:** Tune retry/cooldown knobs from production traffic if upstream providers need different backoff behavior.
- **Context:** `skip_retry_on_failure=true` still pins bound affinity traffic to the bound channel and returns its error; streaming requests retry only before a successful upstream stream begins.

## [2026-05-19 13:03] Leaderboard Redesign
- **Changes:** Redesigned the Vue leaderboard tab with typed ranking rows, summary metrics, ranked provider/consumer boards, score bars, anonymity labels, empty states, and responsive mobile layout.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Backend leaderboard semantics are unchanged; verification used a temporary `/tmp` SQLite database with seeded demo ledger rows for visual inspection.

## [2026-05-19 13:31] Embedded Frontend Assets
- **Changes:** Embedded the built Vue console assets into the Rust binary with `rust-embed`, replaced the runtime `ServeDir` fallback with an embedded asset handler, removed the `TOKENALTAR_FRONTEND_DIST` runtime config, and added an integration test for SPA fallback from embedded assets.
- **Status:** Completed
- **Next Steps:** Run `pnpm --dir frontend build` before compiling Rust whenever the console changes so the binary includes fresh assets.
- **Context:** Runtime deployment no longer needs a `frontend/dist` directory; release smoke test passed from a temporary directory containing only the copied binary and SQLite database.

## [2026-05-19 14:22] Login Hero Redesign
- **Changes:** Redesigned the logged-out Vue hero into a full-width immersive TokenAltar entrance, removed the auth-page sidebar brand block, simplified the hero copy to the product name and short mode labels, and restyled the login/register card for desktop and mobile.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Login/register bindings and API calls are unchanged; verification used `pnpm --dir frontend build` plus Playwright desktop/mobile screenshots.

## [2026-05-19 15:17] Login Email Prefill Removal
- **Changes:** Cleared the logged-out hero login form's default email value so the username field starts empty.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Login autocomplete and API payload bindings are unchanged; this only removes the UI seed value.

## [2026-05-19 16:37] Runtime Economy Configuration
- **Changes:** Added DB-backed runtime settings for seed balances, pricing fallback, settlement rounding, surge multipliers, routing retry/cooldown/weight knobs, queue/cache capacities, and console key/channel defaults; wired backend gateway/routing/pricing paths and the Vue Settings tab to those values.
- **Status:** Completed
- **Next Steps:** Restart the service after changing startup-sized settings such as ledger queue capacity or affinity cache capacity.
- **Context:** Request-time settings apply from `system_settings`; global/channel model price rows still override fallback prices before the configured fallback is used.

## [2026-05-19 17:57] Admin User Management
- **Changes:** Added user enable/disable schema, admin-only user CRUD/reset APIs, disabled-user authentication enforcement, automatic API key/channel shutdown on account suspension, and a Vue Users tab for account management.
- **Status:** Completed
- **Next Steps:** Add audit-log rows for future compliance-grade account operations if needed.
- **Context:** Disabled accounts preserve ledger/resource history but cannot log in or use existing sessions/API keys; the last enabled admin and current admin session are protected from self-lockout.

## [2026-05-19 18:49] Semantic Empty Reply Retry
- **Changes:** Added semantic-empty reply detection for gateway responses and streams, releases failed-channel reservations, applies cooldown, retries fallback channels before client-visible semantic content, and documents the stream boundary.
- **Status:** Completed
- **Next Steps:** Tune the semantic detector if future gateway support adds non-text multimodal assistant outputs beyond the current text/tool surface.
- **Context:** Whitespace-only text deltas, heartbeat/comment frames, usage-only frames, and terminal markers are empty; once text or tool-call semantics are forwarded, the gateway does not replay the stream.
## [2026-05-19 19:40] Passive Channel Health Windows
- **Changes:** Added `channel_health_events` storage, passive gateway health event recording, time-window aggregation on public channel reads, and a compact channel health strip in the console. Documented the passive window semantics in `README.md` and added gateway integration coverage for empty-response windows and success TTFT.
- **Status:** Completed
- **Next Steps:** Continue watching real traffic to tune window size or retention if the console needs a longer historical horizon.
- **Context:** Health windows are fixed 30-minute UTC buckets over the most recent 24 hours. TTFT averages include successful non-empty events only; failed, degraded, and empty events affect status color/counts but not TTFT.

## [2026-05-19 22:10] Dedicated Channel Health Page
- **Changes:** Added a standalone Health console tab that displays all visible channels with provider badges, owner labels for admins, current passive status, 48 health windows, and 24-hour TTFT/sample summaries. Updated `README.md` to document the dedicated view.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** The page reuses `/api/channels` and the existing passive `health_windows` payload, so no new active probing or backend route was added.

## [2026-05-19 22:35] Economy History UI Polish
- **Changes:** Replaced the economy tab's transfer and red-packet history tables with compact responsive cards, empty states, signed transfer amounts, packet progress bars, and mobile-first stacking.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Verified with `pnpm --dir frontend build`, `cargo test`, `git diff --check`, and Playwright desktop/mobile screenshots against a temporary local backend.

## [2026-05-19 22:58] Console Live Updates
- **Changes:** Added an authenticated `/api/events` SSE stream, an in-process console event bus, mutation-triggered topic invalidations for ledger, channel health, settings, user, pricing, and economy changes, and a Vue fetch-stream subscriber that reloads only affected panels.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Events carry resource topics instead of snapshots, so existing REST endpoints remain the permission boundary for owner-scoped and admin-only console data.

## [2026-05-19 23:20] Surge Capacity Semantics
- **Changes:** Changed surge pressure to compare last-hour token demand against primary quota-window capacity normalized to tokens per hour, added `no_capacity` dashboard state without peak pricing, and updated the console label/docs.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Raw remaining quota is no longer used as the denominator for rolling one-hour demand; per-request settlement now carries the pre-reservation surge multiplier so reservation deductions do not price the same request.

## [2026-05-19 23:39] Auth Card Simplification
- **Changes:** Removed redundant logged-out auth card labels, collapsed login/register into a compact header mode switch, converted auth actions to standard forms, and tightened desktop/mobile card spacing.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Verified with `pnpm --dir frontend build` and Playwright desktop/mobile screenshots for login and mobile register states; no API behavior changed.

## [2026-05-20 00:02] Console Page Backgrounds
- **Changes:** Renamed and moved root page background images into `frontend/public/backgrounds`, mapped available images to dashboard, API keys, health, pricing, economy, leaderboards, and settings tabs, and increased the console background layer visibility.
- **Status:** Completed
- **Next Steps:** Add dedicated artwork for users, channels, affinity, and ledger if those tabs need page-specific backgrounds too.
- **Context:** Source files were PNG data despite `.webp` names, so the asset extensions were corrected during the move. Unmapped tabs continue to use the generic TokenAltar background.

## [2026-05-20 00:14] Visual Project Guide
- **Changes:** Moved the user-facing relief guide image into `frontend/public/guides`, added a Guide console tab for all signed-in users, rendered the full image in a responsive framed panel, and documented the guide in `README.md`.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** The guide is content rather than a background, so it lives under `guides` and links to the original image for full-size viewing.

## [2026-05-20 12:45] API Key Channel Selection
- **Changes:** Added `api_key_channels` persistence, route-channel API output, gateway route filtering per key, and a two-column draggable channel picker in the API key console.
- **Status:** Completed
- **Next Steps:** Use the API Keys tab to narrow keys that should not access the whole route pool.
- **Context:** New keys default to all current route channels; keys that still cover the full pool are auto-granted newly created channels, while manually narrowed keys stay narrowed.

## [2026-05-20 13:23] Compact API Key Channel Cards
- **Changes:** Compressed API key channel picker cards to a tighter name/action, provider/status/model, and health-strip layout while removing owner, TTFT, quota, and separate model rows from the visible card body.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Verified with `pnpm --dir frontend build` and Playwright against a temporary local backend; cards now measure about 51px high with the health strip retained.

## [2026-05-20 13:32] Channel Owner Display Names
- **Changes:** Added `owner_display_name` to public channel responses, surfaced provider user names in the Health page and API key channel picker, and documented the response field in `README.md`.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Verified with `cargo test`, `pnpm --dir frontend build`, and Playwright against a temporary local backend with a non-admin-owned channel.

## [2026-05-20 13:48] Frontend Modularization
- **Changes:** Split console types/API access, formatting helpers, health helpers, tab metadata, SSE event handling, and reusable provider/health components out of `frontend/src/App.vue`.
- **Status:** Completed
- **Next Steps:** Continue reducing page-level template size by extracting feature pages such as API Keys, Channels, Pricing, and Settings.
- **Context:** Preserved the current tab-driven SPA and single-binary deployment model; verified with frontend build, Rust tests, Clippy, and release build.

## [2026-05-20 14:04] Affinity Model Presets
- **Changes:** Added migration-seeded GPT, Claude, and Gemini affinity presets, introduced model-name scoping control for affinity cache keys, exposed the control in the console form, documented the preset semantics, and added regression coverage.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Presets follow new-api locality defaults by omitting model names from their cache keys while retaining model-scoped keys for manually created rules by default.

## [2026-05-20 15:59] 1M Model Pricing Presets
- **Changes:** Migrated model and ledger price columns to per-1M-token semantics, fixed `pricing_unit_tokens` at 1,000,000, added GPT-5.5/GPT-5.x and Claude Opus/Sonnet/Haiku global price presets, and exposed the same presets in the pricing form.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Claude cache write tiers are not modeled separately; the single cache price field maps to cached input/cache-hit pricing.

## [2026-05-20 17:01] Guide Mechanism Rulebook
- **Changes:** Added an English, user-facing rulebook section below the Guide relief image with Roman-column styling and dynamic wording for global membership, capacity, and pricing policy without exposing raw keys, channel inventory, quotas, prices, or multipliers.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Verified with `pnpm --dir frontend build`, `cargo test`, `cargo clippy -- -D warnings`, `cargo build --release`, `git diff --check`, and Playwright desktop/mobile Guide screenshots against a temporary SQLite database.

## [2026-05-20 17:16] README Product Polish
- **Changes:** Reworked `README.md` into a polished product-facing project page, placed the Guide PNG banner below the title, reorganized feature, mechanism, console, gateway, operations, configuration, and verification sections, and refreshed wording for current routing/pricing/health behavior.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Verified with `pnpm --dir frontend build`, `git diff --check`, and explicit checks that `frontend/public/guides/tokenaltar-project-guide.png` and the built `frontend/dist/guides/tokenaltar-project-guide.png` exist.

## [2026-05-20 17:31] Project Logo Integration
- **Changes:** Moved the supplied root `logo.png` into `frontend/public/logo.png`, used it in the README header, wired it as the browser favicon and Apple touch icon, updated the app title, and replaced the sidebar TA badge with the logo image.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Verified with `pnpm --dir frontend build`, `git diff --check`, a built `frontend/dist/logo.png`, and Playwright checks for `/logo.png`, document favicon links, and loaded sidebar logo dimensions.

## [2026-05-20 18:10] Global Provider Share Governance
- **Changes:** Removed provider-share editing from channel owner inputs and the Vue channel form, made channel create/update use the admin-configured global provider share, and made gateway settlement calculate provider points from the runtime global share rather than per-channel values.
- **Status:** Completed
- **Next Steps:** Rebuild/deploy the embedded frontend bundle so the channel form no longer exposes provider-share controls.
- **Context:** Backward-compatible JSON submissions with `provider_share` are ignored by Serde; regression tests cover ignored channel updates and global-share settlement.

## [2026-05-20 18:31] Docker Image Publishing
- **Changes:** Added a multi-stage Dockerfile, Docker ignore rules, Docker Compose runtime config, `.env.example`, and a GitHub Actions workflow that builds and pushes GHCR images from `main`, `v*` tags, and manual runs. Documented the container deployment path in `README.md`.
- **Status:** Completed
- **Next Steps:** Configure production admin credentials before first container startup and confirm repository package write permissions if GHCR push fails.
- **Context:** The container build still runs the Vue build before Rust compilation so `frontend/dist` is embedded into the release binary; runtime SQLite data is expected at `/data/tokenaltar.sqlite3`.

## [2026-05-20 19:19] Channel Quota Points
- **Changes:** Migrated channel quota windows from token counters to point counters, updated gateway reservation/settlement/routing/surge logic to use point capacity, refreshed console/API field names, and documented the `limit_points` / `used_points` payload.
- **Status:** Completed
- **Next Steps:** Deploy the new migration and rebuilt embedded frontend together so existing token quotas are converted to point quotas before the console uses the new fields.
- **Context:** Migration `0014_channel_quota_points.sql` converts old token quotas using the active fallback input price per pricing unit, then updates untouched default window JSON to point limits.

## [2026-05-20 20:03] Settings Save Scalar Values
- **Changes:** Made `/api/settings` accept string, number, and boolean scalar setting values by normalizing them to strings before existing validation, and made the Vue settings form stringify values before submission.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Fixes Axum JSON deserialization failures such as `value: invalid type: integer 50, expected a string` while preserving validation and rejecting non-scalar setting values.

## [2026-05-20 20:24] Provider Payout Multiplier
- **Changes:** Changed `default_channel_provider_share` validation from ratio-only to any non-negative finite multiplier, renamed the Settings label to Provider Payout Multiplier, documented the wording, and added settlement coverage for a `1.25` payout.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** Provider settlement still uses `provider_points = total_points * default_channel_provider_share`; values above `1.0` now intentionally represent subsidized provider payout.

## [2026-05-20 20:46] Stable Docker Volume Name
- **Changes:** Added an explicit Docker Compose volume name for `tokenaltar-data` and documented how to migrate data from older project-prefixed Compose volumes.
- **Status:** Completed
- **Next Steps:** Migrate any existing `*_tokenaltar-data` volume into `tokenaltar-data` before starting the updated stack.
- **Context:** Compose otherwise prefixes named volumes with the project name, so changing checkout directories or project names can silently create a fresh SQLite volume.

## [2026-05-20 21:22] Route Quota Reserve Fit
- **Changes:** Updated gateway routing to compute each candidate channel's estimated reserve with its effective price book, and changed channel selection to skip channels whose quota windows cannot cover that estimate. Added unit and gateway regression coverage for the small-remaining-quota fallback path.
- **Status:** Completed
- **Next Steps:** None.
- **Context:** The transactional reservation check remains the final concurrent safety guard; routing now avoids picking known-underfilled candidates before that transaction.
