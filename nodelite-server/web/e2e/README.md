# NodeLite E2E baseline

This directory holds Playwright tests that exercise the **legacy UI** (the existing `assets/index.html` + `assets/node.html`).
They are the regression baseline for the Vue + Vite migration (see [docs/frontend-vue-refactor-plan.md](../../../docs/frontend-vue-refactor-plan.md) §3.7).

## Running

```bash
# 1) Start the server in another terminal
cargo run -p nodelite-server

# 2) Provide credentials and run
NODELITE_E2E_BASE_URL=http://localhost:8080 \
NODELITE_E2E_USER=admin \
NODELITE_E2E_PASS=changeme \
pnpm --dir nodelite-server/web e2e
```

`NODELITE_E2E_BASE_URL` defaults to `http://localhost:8080`.
`NODELITE_E2E_USER`/`NODELITE_E2E_PASS` map to Playwright's `httpCredentials` so Basic Auth is sent automatically.

## Coverage targets (12 baseline flows)

The full list lives in the plan, §3.7.2. Each `*.spec.ts` file in this directory implements one flow.
All 12 stubs are checked in and marked `test.fixme()` until they're recorded against the legacy backend — Playwright will list them but not run them, so `pnpm e2e` stays green while implementation is pending.

| # | File | Flow |
|---|---|---|
| 1 | `login-basic-auth.spec.ts` | Login (Basic Auth) → dashboard loads |
| 2 | `verify-2fa.spec.ts` | Wrong TOTP rate-limits; correct TOTP redirects |
| 3 | `auto-logout-24h.spec.ts` | 24h timer triggers `/logout-and-reauth` |
| 4 | `theme-toggle.spec.ts` | dark ↔ light + localStorage persistence |
| 5 | `language-toggle.spec.ts` | en ↔ zh updates all i18n-bound copy |
| 6 | `node-list-navigation.spec.ts` | Card click → `/nodes/:id` with WS preserved |
| 7 | `node-detail-tabs.spec.ts` | overview / monitor / network / logs |
| 8 | `chart-interaction.spec.ts` | hover tooltip + modal open/close |
| 9 | `settings-change-password.spec.ts` | Mutation triggers reauth challenge |
| 10 | `alert-settings.spec.ts` | Channel + rule CRUD (post-reauth) |
| 11 | `map-node-location.spec.ts` | Marker click highlights + jumps to detail |
| 12 | `ws-reconnect.spec.ts` | Drop → reconnect indicator → recovery |

To implement a flow, replace `test.fixme(...)` with `test(...)` and fill in the TODOs. The plan §3.7.4 documents the accepted compromises (no pixel diffs, chart contents asserted via tooltip text).
