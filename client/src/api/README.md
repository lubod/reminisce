# API Client Configuration (`client/src/api/`)

## Purpose
Configures the centralized Axios HTTP client instance used by MobX stores for backend API communication.

## Interceptors & Authentication Flow
- **Base Config**: `baseURL` is fixed to `/api` and `withCredentials` is enabled.
- **Request Interceptor**: The no-op request interceptor exists as a seam; authentication is delegated to the backend's `HttpOnly`, `SameSite` session cookie set on login. No JWT is read from or written to `localStorage`, so tokens are not exposed to XSS or the DOM.
- **Response Interceptor (401 Auto-Handling)**:
  - Listens for HTTP 401 Unauthorized responses.
  - Excludes login and identity verification endpoints (`user-login`, `/auth/me`, `/login`) to avoid infinite redirect loops on invalid credentials.
  - Redirects to `/login` when the session cookie expires or is invalid.

## Architectural Convention
- **No Per-Endpoint API Modules**: Do not create separate API abstraction files (e.g. `api/media.ts`, `api/users.ts`). MobX stores import the default Axios `instance` from `api/axiosConfig.ts` and make HTTP calls directly. This keeps API payload types tied directly to store actions.

## Key Files
- [axiosConfig.ts](file:///Users/ldr/work/reminisce/client/src/api/axiosConfig.ts): Primary Axios instance exported for application use.

## Invariants & Gotchas
- **Exclusion List**: If adding new auth endpoints that return 401 on failed credential validation, update the exclusion condition in `axiosConfig.ts` to prevent unexpected user redirects.
