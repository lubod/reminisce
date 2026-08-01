# Frontend Source Core (`client/src/`)

## Purpose
Core application code for the Reminisce React client including boot orchestration, routing, global store context, and error boundaries.

## Architecture & Boot Flow
```
main.tsx ──▶ telemetry.ts (OpenTelemetry initialization)
    └──▶ App.tsx ──▶ RootStore Context Provider
             └──▶ authStore.initialize() (verify JWT token)
                      └──▶ React Router
                               ├── /login ──▶ LoginForm
                               └── /*     ──▶ ProtectedRoute ──▶ Layout & Dashboard
```

1. **Bootstrap**: `main.tsx` initializes OpenTelemetry logging/tracing before rendering `App.tsx`.
2. **State Context**: `App.tsx` wraps the UI tree in `StoreContext.Provider` supplying the `RootStore` instance accessible via `useStore()`.
3. **Authentication**: `authStore.initialize()` validates the backend session. Because the session is an `HttpOnly` cookie (not a `localStorage` JWT), re-authentication on page reload goes through the cookie-authenticated `GET /auth/me`.
4. **Routing**: `ProtectedRoute` gates authenticated routes, rendering `Layout.tsx` with sidebar navigation.

## HttpOnly-Cookie Authentication Model
- **Session Cookie**: Login/setup set an `HttpOnly`, `SameSite=Lax` (Secure in production) `access_token` cookie. Axios (`withCredentials: true`) and native `<img>`/`<video>` same-origin requests send it automatically. JWTs are kept in memory only (`AuthStore`), never in `localStorage`, and never in URL query strings — keeping them out of XSS reach, nginx logs, history, and `Referer` headers.
- **Media URLs**: Media/thumbnail URLs must NOT append `?token=` — they are authenticated by the session cookie.
- **Logout**: `user_logout` clears the cookie; `AuthStore` clears in-memory state.

## Three-Tier Error Handling
1. **Tier 1 (Network / Auth)**: Axios response interceptor in `api/` catches HTTP 401 unauthenticated errors and auto-redirects to `/login`. Transient failures (network/5xx) do NOT log the user out.
2. **Tier 2 (Domain Errors)**: MobX stores catch API errors, set observable `error` state properties, and push notification toasts to `UIStore`.
3. **Tier 3 (UI Crash Boundary)**: `ErrorBoundary.tsx` wraps component trees to catch uncaught React rendering exceptions with a fallback error view.

## Subdirectories
- [api/](file:///Users/ldr/work/reminisce/client/src/api): Axios HTTP client instance and request/response interceptors.
- [components/](file:///Users/ldr/work/reminisce/client/src/components): React UI components grouped by feature area.
- [stores/](file:///Users/ldr/work/reminisce/client/src/stores): MobX reactive state stores and `RootStore`.
- [types/](file:///Users/ldr/work/reminisce/client/src/types): Shared TypeScript interfaces for API DTOs and state objects.
- [utils/](file:///Users/ldr/work/reminisce/client/src/utils): Helper utilities and loggers.

## Invariants & Gotchas
- **Hook Access**: Always access stores via `useStore()` hook rather than instantiating store classes directly in components.
- **Media URLs**: Media/thumbnail URLs must rely on the session cookie for auth — never append `?token=` (it would leak credentials into logs/history/Referer).
- **Stale Responses**: Do not remove the `requestSeq` stale-response guard in `MediaStore`; out-of-order search/filter responses must never overwrite newer results.
