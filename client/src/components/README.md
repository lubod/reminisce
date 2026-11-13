# Frontend React Components (`client/src/components/`)

## Purpose
React UI component inventory. Components render the visual interface, handle user interaction, and bind to reactive state stores.

## Architectural Conventions
- **Shallow Component Structure**: Feature components live directly in `client/src/components/`, with decomposed sub-views organized cleanly into feature folders (`Dashboard/`, `Lightbox/`, `System/`).
- **MobX Observer Pattern**: Components reading store properties MUST be wrapped with MobX's `observer(...)` HOC from `mobx-react-lite` to reactively re-render when state changes.
- **Kiosk / Hide Menu Mode**: `Layout.tsx` checks the `?hidemenu=true` URL query parameter to render headerless/sidebarless layouts for kiosk or wall-display presentations.

## Component Inventory by Feature Domain

### Shell & Auth
- [LoginForm.tsx](file:///Users/ldr/work/reminisce/client/src/components/LoginForm.tsx): User authentication form (`AuthStore`).
- [Layout.tsx](file:///Users/ldr/work/reminisce/client/src/components/Layout.tsx): Main app layout containing sidebar, header, and content outlet (`UIStore`).
- [ErrorBoundary.tsx](file:///Users/ldr/work/reminisce/client/src/components/ErrorBoundary.tsx): Catch-all React error boundary displaying emergency fallback UI.

### Dashboard & Analytics
- [Dashboard.tsx](file:///Users/ldr/work/reminisce/client/src/components/Dashboard.tsx): Tabbed dashboard coordinator rendering sub-views in [Dashboard/](file:///Users/ldr/work/reminisce/client/src/components/Dashboard) (`StatsStore`, `MediaStore`, `SystemStore`).
- [PresentationMode.tsx](file:///Users/ldr/work/reminisce/client/src/components/PresentationMode.tsx): Fullscreen automated slideshow mode for gallery media (`MediaStore`).

### Media Gallery, Map & Import
- [MediaBrowser.tsx](file:///Users/ldr/work/reminisce/client/src/components/MediaBrowser.tsx): Main virtualized/paginated photo and video gallery grid (`MediaStore`).
- [MediaLightbox.tsx](file:///Users/ldr/work/reminisce/client/src/components/MediaLightbox.tsx): Fullscreen media viewer coordinating [Lightbox/](file:///Users/ldr/work/reminisce/client/src/components/Lightbox) subcomponents (`LightboxViewport.tsx`, `LightboxSidebar.tsx`, `LightboxEnhancePanel.tsx`).
- [MapView.tsx](file:///Users/ldr/work/reminisce/client/src/components/MapView.tsx): Clustered geotagged map browser using Supercluster & Leaflet.
- [OrientationCheck.tsx](file:///Users/ldr/work/reminisce/client/src/components/OrientationCheck.tsx): Orientation batch inspection/correction view.
- [ServerImportModal.tsx](file:///Users/ldr/work/reminisce/client/src/components/ServerImportModal.tsx): Dialog for triggering server-side directory media ingestion (`MediaStore`).
- [DirectoryImportModal.tsx](file:///Users/ldr/work/reminisce/client/src/components/DirectoryImportModal.tsx): Browser local directory import dialog (`MediaStore`).

### System Observability
- [System/](file:///Users/ldr/work/reminisce/client/src/components/System): In-app diagnostics (`AiModelsViewer.tsx`, `Charts.tsx`, `ErrorsViewer.tsx`, `LogViewer.tsx`, `P2PRebalanceProgress.tsx`).

### People & Faces
- [People.tsx](file:///Users/ldr/work/reminisce/client/src/components/People.tsx): Overview grid of recognized faces and clusters (`PersonStore`).
- [PersonDetail.tsx](file:///Users/ldr/work/reminisce/client/src/components/PersonDetail.tsx): Person detail page with face naming & merge actions (`PersonStore`).
- [PersonGallery.tsx](file:///Users/ldr/work/reminisce/client/src/components/PersonGallery.tsx): Media grid filtered by a specific person (`PersonStore`).

### Duplicates & Trash
- [DuplicatesBrowser.tsx](file:///Users/ldr/work/reminisce/client/src/components/DuplicatesBrowser.tsx): List view of detected duplicate image candidate pairs (`DuplicatesStore`).
- [DuplicatesLightbox.tsx](file:///Users/ldr/work/reminisce/client/src/components/DuplicatesLightbox.tsx): Side-by-side comparison modal for keeping/deleting duplicates (`DuplicatesStore`).
- [TrashBrowser.tsx](file:///Users/ldr/work/reminisce/client/src/components/TrashBrowser.tsx): Soft-deleted media bin with restore actions (`TrashStore`).

### System & Admin
- [UserManagement.tsx](file:///Users/ldr/work/reminisce/client/src/components/UserManagement.tsx): Admin user creation and role configuration interface (`AuthStore`).
- [AndroidConnectionQR.tsx](file:///Users/ldr/work/reminisce/client/src/components/AndroidConnectionQR.tsx): Modal presenting QR code for pairing mobile devices (`AuthStore`).

## Invariants & Gotchas
- **Always Wrap with `observer`**: Forgetting `observer()` on components that read store values will break reactivity without showing console warnings.
