# Web Client Deployment

This document covers the two deployment modes for the OpenSymphony web client.

## Deployment Modes

### 1. Gateway-Served (Default)

The built web app static assets are served directly by the OpenSymphony Gateway
under the `/app/` prefix. This is the default mode for local and external
gateway deployments.

```
┌──────────┐     ┌─────────────────────────────────────────┐
│ Browser  │────>│  OpenSymphony Gateway                   │
│          │     │  /api/v1/*   → API endpoints             │
│          │<────│  /app/*      → Static web assets         │
│          │     │  /app/        → index.html (SPA root)    │
└──────────┘     └─────────────────────────────────────────┘
```

**Setup:**

1. Build the web app:
   ```bash
   npm run build --workspace=@opensymphony/web
   ```
   This produces `apps/web/dist/` with `index.html` and `assets/`.

2. Configure the gateway to serve the built assets:
   ```rust
   let server = GatewayServer::new(store)
       .with_web_assets("apps/web/dist");
   ```

3. The gateway now serves the web app at `/app/` alongside the API endpoints.

**Key characteristics:**
- Single origin for the web app and API (no CORS required).
- The web app resolves the gateway URL as the current origin.
- `VITE_APP_BASE_PATH` defaults to `/app/`.
- Cache-busted asset filenames are enabled by default (Vite build).

### 2. Separately Deployed

The web app is deployed as independent static assets on a separate server or
CDN, and configured to point at a remote gateway URL.

```
┌──────────┐     ┌──────────────┐     ┌──────────────────┐
│ Browser  │────>│  Static Host │     │  Gateway Server  │
│          │     │  (CDN/nginx) │────>│  /api/v1/*       │
│          │<────│  index.html  │     │   (SSE)   │
└──────────┘     └──────────────┘     └──────────────────┘
```

**Setup:**

1. Build the web app with the target gateway URL:
   ```bash
   VITE_GATEWAY_URL=https://gateway.example.com \
   VITE_APP_BASE_PATH=/ npm run build --workspace=@opensymphony/web
   ```

2. Deploy `apps/web/dist/` to a static file host (nginx, Cloudflare Pages, etc.).

3. Configure CORS on the gateway to allow the web app origin.

**Environment Variables:**

| Variable | Description | Default |
|---|---|---|
| `VITE_GATEWAY_URL` | Gateway base URL for API calls. Set for separately deployed mode. | (empty = same origin) |
| `VITE_APP_BASE_PATH` | Base path for static assets in the HTML. | `/app/` |
| `VITE_DEV_GATEWAY_URL` | Gateway URL for the Vite dev-server proxy. | `http://127.0.0.1:3000` |

## Local Development

Run the Vite dev server with proxy support:

```bash
VITE_DEV_GATEWAY_URL=http://127.0.0.1:3000 npm run dev --workspace=@opensymphony/web
```

The dev server proxies `/api/*` to the local gateway
URL, allowing the browser app to communicate with the gateway without CORS
issues during development.

## Gateway Base URL Configuration

When deploying separately, the gateway URL must be known at build time (Vite
`define` injection) or provided at runtime via `VITE_GATEWAY_URL`.

**Build-time injection (recommended for production):**
```bash
VITE_GATEWAY_URL=https://gateway.example.com npm run build --workspace=@opensymphony/web
```

**Runtime configuration:**
The web app checks `import.meta.env.VITE_GATEWAY_URL` first, then falls back
to the compile-time `__GATEWAY_URL__` constant, and finally defaults to the
same origin.

## Build Verification

Run the type check and existing tests:

```bash
npm run type-check  # TypeScript type check across all packages
npm test            # Jest tests
npm run build --workspace=@opensymphony/web  # Vite build
```

After building, `apps/web/dist/` should contain:
- `index.html` - Entry point with correct base path.
- `assets/` - Cache-busted CSS and JS bundles.
## No Tauri Dependencies

The web build does not include Tauri APIs or desktop-only transports. The
`apps/web/src/` directory is isolated from `apps/desktop/src/` and only imports
shared packages from `packages/`.

