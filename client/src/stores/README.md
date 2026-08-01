# Frontend MobX Stores (`client/src/stores/`)

## Purpose
State management layer implementing reactive state stores powered by MobX and MobX React Lite.

## Store Architecture & Conventions
- **RootStore Singleton**: `RootStore` instantiates all sub-stores and holds references to them (`this.authStore`, `this.mediaStore`, etc.), enabling cross-store interaction via constructor injected `rootStore`.
- **MobX Action Invariants**:
  - Class constructors invoke `makeAutoObservable(this)`.
  - Async state mutations following `await` MUST be wrapped in `runInAction(() => { ... })` to satisfy MobX strict mode rules.
- **Error Routing**: Domain stores capture API errors and push global notification banners via `rootStore.uiStore.showToast(...)` or set component-visible error states.

## Store Inventory (9 Stores)

| Store | Role |
|-------|------|
| [RootStore.ts](file:///Users/ldr/work/reminisce/client/src/stores/RootStore.ts) | Central container instantiating and wiring all sub-stores |
| [AuthStore.ts](file:///Users/ldr/work/reminisce/client/src/stores/AuthStore.ts) | Session state (HttpOnly cookie), current user identity, authentication status. JWT/imageToken are kept in memory only — never localStorage |
| [MediaStore.ts](file:///Users/ldr/work/reminisce/client/src/stores/MediaStore.ts) | Media gallery grid state, pagination, filtering, search criteria, selection |
| [PersonStore.ts](file:///Users/ldr/work/reminisce/client/src/stores/PersonStore.ts) | Face recognition clusters, person naming, and face assignment |
| [LabelStore.ts](file:///Users/ldr/work/reminisce/client/src/stores/LabelStore.ts) | Tagging system, custom labels, and label assignment |
| [DuplicatesStore.ts](file:///Users/ldr/work/reminisce/client/src/stores/DuplicatesStore.ts) | Near-duplicate image pairs, similarity scores, and review resolution |
| [StatsStore.ts](file:///Users/ldr/work/reminisce/client/src/stores/StatsStore.ts) | Dashboard metrics, hardware storage stats, and database telemetry |
| [TrashStore.ts](file:///Users/ldr/work/reminisce/client/src/stores/TrashStore.ts) | Soft-deleted media items, restoration, and permanent purge actions |
| [UIStore.ts](file:///Users/ldr/work/reminisce/client/src/stores/UIStore.ts) | Global UI state (toast alerts, sidebar visibility, theme, active modal) |

## Invariants & Gotchas
- **Reaction Debouncing**: `MediaStore` uses MobX reactions to automatically refetch gallery pages when filter options change. Do not invoke manual refetch loops inside UI components.
- **Blob Memory Cleanup**: When generating `URL.createObjectURL(...)` for local media previews, always revoke object URLs upon component unmount or preview update (`URL.revokeObjectURL(url)`).
