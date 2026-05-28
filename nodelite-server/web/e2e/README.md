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
Stage 0 ships an empty harness and the smoke test only; the remaining flows land in subsequent commits.
