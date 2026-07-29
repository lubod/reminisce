# Client Application (`client/`)

## Purpose
Frontend web application for Reminisce, built as a modern Single Page Application (SPA) for browsing, searching, and managing self-hosted photo/video libraries.

## Tech Stack
- **Framework**: React 19 + TypeScript
- **Build Tool**: Vite 5
- **State Management**: MobX (reactive stores with MobX React Lite)
- **Styling**: Tailwind CSS 4

## Development & Proxy Setup
During development (`npm run dev` running on `http://localhost:5173`), Vite proxies API requests matching `/api` to the backend server:
- Dev proxy target: `http://localhost:8080` (configured in `vite.config.ts`).
- Production deployment: Nginx handles frontend static file serving and proxies `/api` requests to `reminisce` binary.

## Common Commands
All frontend commands must be executed inside the `client/` directory:
```bash
npm install     # Install dependencies
npm run dev     # Start Vite dev server with proxy
npm run build   # TypeScript check & production build (dist/)
npm test        # Unit tests (vitest + jsdom)
```

Unit tests use **vitest** with the `jsdom` environment (config: `vitest.config.ts`, separate from `vite.config.ts` so the OpenTelemetry node stubs don't interfere). Store logic tests live next to stores as `*.test.ts` (e.g. `src/stores/MediaStore.test.ts`). **Note:** stores import `RootStore` via `import type` to avoid a runtime circular dependency (`RootStore` instantiates the stores) — keep it type-only.

## Directory Structure
- [src/](file:///Users/ldr/work/reminisce/client/src): React component hierarchy, MobX stores, API client, and application entry point.

## Invariants & Gotchas
- **Run in `client/`**: Never run `npm` or Vite commands from the workspace root. Always `cd client` first.
- **Proxy Parity**: API calls must always use `/api/...` relative paths so development proxying matches production Nginx routing.
