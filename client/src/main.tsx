import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'

// Lazy-load OpenTelemetry only when explicitly enabled so default bundles stay slim
if (import.meta.env.VITE_ENABLE_OTEL === 'true') {
  import('./telemetry').then(({ default: setupTelemetry }) => setupTelemetry());
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
