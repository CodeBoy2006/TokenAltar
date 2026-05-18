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
