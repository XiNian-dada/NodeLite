# NodeLite E2E baseline

This directory holds Playwright tests for the Vue + Vite UI. Most specs stub the
small API surface they need so they can run against Vite without a Rust backend.
Backend-only flows can still target a live server with `NODELITE_E2E_BASE_URL`.

## Running

```bash
# Vite is started automatically by Playwright when NODELITE_E2E_BASE_URL is unset.
pnpm --dir nodelite-server/web e2e

# To run against a live backend instead:
cargo run -p nodelite-server
NODELITE_E2E_BASE_URL=http://localhost:8080 \
NODELITE_E2E_USER=admin \
NODELITE_E2E_PASS=changeme \
pnpm --dir nodelite-server/web e2e
```

`NODELITE_E2E_BASE_URL` defaults to Vite at `http://127.0.0.1:5173`.
`NODELITE_E2E_USER`/`NODELITE_E2E_PASS` map to Playwright's `httpCredentials` so Basic Auth is sent automatically.

## Coverage targets (12 baseline flows)

The full list lives in the plan, §3.7.2. Each `*.spec.ts` file in this directory implements one flow.
The UI-only flows run with local fixtures. WebSocket reconnect flows require a
live backend and are skipped unless `NODELITE_E2E_BASE_URL` is set.

| # | File | Flow |
|---|---|---|
| 1 | `login-basic-auth.spec.ts` | Login (Basic Auth) → dashboard loads |
| 2 | `verify-2fa.spec.ts` | Wrong TOTP rate-limits; correct TOTP redirects |
| 3 | `auto-logout-24h.spec.ts` | 24h timer triggers `/logout-and-reauth` |
| 4 | `theme-toggle.spec.ts` | dark ↔ light + localStorage persistence |
| 5 | `language-toggle.spec.ts` | en ↔ zh updates all i18n-bound copy |
| 6 | `node-list-navigation.spec.ts` | Card click → `/nodes/:id` |
| 7 | `node-detail-tabs.spec.ts` | overview / monitor / network / logs |
| 8 | `chart-interaction.spec.ts` | hover tooltip + modal open/close |
| 9 | `settings-change-password.spec.ts` | Mutation posts reauth credentials |
| 10 | `alert-settings.spec.ts` | Channel + rule CRUD (post-reauth) |
| 11 | `map-node-location.spec.ts` | Marker click highlights + jumps to detail |
| 12 | `ws-reconnect.spec.ts` | Drop → reconnect indicator → recovery |

The plan §3.7.4 documents the accepted compromises (no pixel diffs, chart contents asserted via tooltip text).
