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
