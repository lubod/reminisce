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
3. **Authentication**: `authStore.initialize()` checks local storage for saved tokens and validates sessions.
4. **Routing**: `ProtectedRoute` gates authenticated routes, rendering `Layout.tsx` with sidebar navigation.

## Dual-Token Authentication Model
- **Header Auth**: Standard API calls append `Authorization: Bearer <token>` automatically via the Axios interceptor in `api/axiosConfig.ts`.
- **Query Param Auth (`imageToken`)**: Native HTML elements (`<img src="...">`, `<video src="...">`) cannot send HTTP headers. The backend accepts `?token=` for thumbnail/media endpoints. `AuthStore` maintains `imageToken` for embedding image URLs directly in JSX.

## Three-Tier Error Handling
1. **Tier 1 (Network / Auth)**: Axios response interceptor in `api/` catches HTTP 401 unauthenticated errors, clears tokens, and triggers auto-redirect to `/login`.
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
- **Media URLs**: Media/thumbnail URLs passed to native DOM elements MUST append `?token=${authStore.imageToken}` to avoid 401 image load failures.
