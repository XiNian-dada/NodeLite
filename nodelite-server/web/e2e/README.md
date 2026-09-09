# NodeLite E2E baseline

This directory holds Playwright tests for the Vue + Vite UI. Most specs stub the
small API surface they need so they can run against Vite without a Rust backend.
Live WebSocket flows run against an isolated Rust server through `e2e:live`.

## Running

```bash
# Fixture UI tests start Vite automatically.
pnpm --dir nodelite-server/web e2e

# Live integration: builds the UI and Rust server, then starts an isolated backend.
# Requires Node.js 22+ and a Rust toolchain.
pnpm --dir nodelite-server/web e2e:live
```

The live runner creates random credentials and private temporary config/SQLite
files. Every test enrolls a deterministic Agent through the real installation
API, sends the real WebSocket protocol, and revokes it after the test. Processes
and data are cleaned on success, failure, SIGINT and SIGTERM. It never points at
an existing deployment. To reuse a current build, set `NODELITE_E2E_SERVER_BIN`
to the absolute path of that binary.

CI runs both suites and uploads reports, traces and the server log on failure.
The separate live configuration requires its environment and has no conditional
skips; the runner rejects an empty report or any skipped test. Live reports are
in `playwright-report/live` and `test-results/live*`.

## Coverage targets (14 spec files)

The first 12 flows come from the original plan, §3.7.2. Two supplementary suites
cover the application shell and the WebSocket-first dashboard. UI-only flows run
with local fixtures. Live WebSocket flows run in the separate `e2e:live` command and are required in CI.

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
| 13 | `smoke.spec.ts` | Authenticated application shell and dashboard smoke check |
| 14 | `ws-dashboard.spec.ts` | WS initial state, incremental updates, REST fallback, navigation, and visibility lifecycle |

The plan §3.7.4 documents the accepted compromises (no pixel diffs, chart contents asserted via tooltip text).
