# NodeLite Web UI

The NodeLite web interface is a Vue 3 single-page application built with Vite and TypeScript. It is developed in this directory, compiled to `web/dist/`, and embedded into the `nodelite-server` binary for production releases.

## Technology and runtime model

- **Vue 3** renders the application with Composition API components.
- **Vite** provides the development server and production build.
- **TypeScript** is checked by `vue-tsc` in both the standalone typecheck and production build.
- **Pinia** owns shared API and monitoring state across 10 stores.
- **Vue Router** maps 6 lazy-loaded views to the SPA routes.
- **vue-i18n** loads the server-provided `/assets/ui-i18n.json` dictionary for `en` and `zh-CN`.
- **Vitest** and Vue Test Utils cover components, composables, stores, API clients, and WebSocket behavior.
- **Playwright** exercises browser flows on desktop Chromium and a mobile Chromium profile.

`App.vue` owns one WebSocket client for the lifetime of the SPA. Route views subscribe to typed browser messages and update Pinia stores. The dashboard is WebSocket-first, but starts REST requests after 500 ms if no initial WebSocket state has arrived, so a reconnecting socket does not leave the page empty. Other screens use the typed REST client for history, logs, settings, authentication, and alert-management operations.

## Views

| Route | View | Responsibility |
|---|---|---|
| `/` | `DashboardView.vue` | Live overview statistics, node health matrix, world map, node list, and recent-login notification |
| `/nodes/:id` | `NodeDetailView.vue` | Per-node overview, monitor charts, network and hardware details, logs, and node settings |
| `/settings` | `SettingsView.vue` | Server metadata, update operations, operational controls, and Agent token management |
| `/account` | `AccountView.vue` | Session information, password changes, two-factor authentication, and logout |
| `/alerts` | `AlertsView.vue` | Alert rules, SMTP and webhook channels, inspection reports, previews, and reauthentication |
| `/logs` | `LogsView.vue` | Authentication and administrative audit events |

The current source tree contains 38 Vue components, 10 Pinia stores, and 6 route views. Keep the counts and responsibilities in this document synchronized when adding or removing modules.

## Source layout

```text
web/
├── e2e/                 # Playwright browser flows and helpers
├── public/              # Files copied unchanged into dist/
├── src/
│   ├── api/             # Typed REST client, response types, and fixtures
│   ├── auth/            # Session expiry and authentication helpers
│   ├── components/      # Reusable dashboard, detail, settings, and alert UI
│   ├── composables/     # Polling, charts, theme, maps, and form-state logic
│   ├── i18n/            # Locale loading and language selection
│   ├── lib/             # Formatting, chart, map, and domain helpers
│   ├── router/          # SPA route definitions
│   ├── stores/          # Pinia state for API and monitoring data
│   ├── styles/          # Shared theme and responsive styles
│   ├── views/           # Six route-level views
│   ├── ws/              # Browser WebSocket client and message validation
│   ├── App.vue          # Router shell and WebSocket lifetime owner
│   └── main.ts          # Vue, Pinia, router, and i18n bootstrap
├── index.html           # Vite entry document and early theme/auth bootstrap
├── package.json         # pnpm scripts and frontend dependencies
├── playwright.config.ts # Browser test projects and live-backend options
├── vite.config.ts       # Dev proxy and production bundle settings
└── vitest.config.ts     # Unit/component test configuration
```

## Prerequisites

- Node.js 20.19+ (20.x), 22.13+ (22.x), or 24+, matching `package.json`'s `engines` range
- pnpm 10.11.0 (`corepack enable` is sufficient when Corepack is available)
- A Rust toolchain when running the NodeLite backend

Install the pinned frontend dependencies from the repository root:

```bash
pnpm --dir nodelite-server/web install --frozen-lockfile
```

## Commands

Run these commands from `nodelite-server/web`, or prefix them with `pnpm --dir nodelite-server/web` from the repository root.

| Command | Purpose |
|---|---|
| `pnpm dev` | Start Vite on `http://localhost:5173` and proxy backend routes |
| `pnpm build` | Run the build-mode TypeScript check and create `dist/` |
| `pnpm typecheck` | Run `vue-tsc --noEmit` |
| `pnpm lint` | Run ESLint with warnings treated as failures |
| `pnpm test` | Run all Vitest suites once |
| `pnpm test:watch` | Run Vitest in watch mode |
| `pnpm e2e` | Run Playwright, starting Vite automatically when no base URL is supplied |
| `pnpm e2e:ui` | Open Playwright's interactive test runner |
| `pnpm preview` | Serve the production bundle locally |
| `pnpm format` | Format frontend files with Prettier |

## Local development

Run the Rust API and Vite development server in separate terminals:

```bash
# Terminal A: API, WebSocket, authentication, and runtime assets
cargo run -p nodelite-server

# Terminal B: frontend with hot reload
pnpm --dir nodelite-server/web dev
```

Open `http://localhost:5173` and authenticate with the credentials configured for the server. Vite forwards `/api`, `/ws`, `/assets/ui-i18n.json`, and the authentication helper routes to `http://localhost:8080` by default.

Point the proxy at another backend with `NODELITE_DEV_BACKEND`:

```bash
NODELITE_DEV_BACKEND=http://127.0.0.1:9090 \
  pnpm --dir nodelite-server/web dev
```

## Testing

The frontend currently has 81 `*.spec.ts` Vitest files under `src/`. Run the fast checks during normal development:

```bash
pnpm --dir nodelite-server/web lint
pnpm --dir nodelite-server/web typecheck
pnpm --dir nodelite-server/web test
```

There are 14 Playwright spec files in `e2e/`. Most tests stub the small API surface they need, and `pnpm e2e` starts Vite automatically. To exercise flows against a running NodeLite server instead, provide its URL and optional Basic Auth credentials:

```bash
cargo run -p nodelite-server

NODELITE_E2E_BASE_URL=http://127.0.0.1:8080 \
NODELITE_E2E_USER=admin \
NODELITE_E2E_PASS='configured-password' \
pnpm --dir nodelite-server/web e2e
```

See [`e2e/README.md`](e2e/README.md) for the browser-flow inventory and environment details.

## Production build and embedding

`nodelite-server/build.rs` tracks the frontend source and build configuration. A normal Server build runs:

1. `pnpm --dir web install --frozen-lockfile`
2. `pnpm --dir web build`

Vite writes the production SPA to `web/dist/`. `nodelite-server/src/web_assets.rs` then embeds that directory with `include_dir!`, so the released Server binary serves the UI without a separate static-file deployment.

For backend-only iteration, `NODELITE_SKIP_WEB_BUILD=1` skips the pnpm steps only when a previously built `web/dist/index.html` already exists:

```bash
pnpm --dir nodelite-server/web build
NODELITE_SKIP_WEB_BUILD=1 cargo build -p nodelite-server
```

The `web/dist/` directory is generated and gitignored; do not commit it.
