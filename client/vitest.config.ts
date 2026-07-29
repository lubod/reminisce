import { defineConfig } from 'vitest/config'
import path from 'path'

// Vitest runs store/logic unit tests in a DOM-like environment (jsdom provides
// localStorage + window that some modules touch). Kept separate from vite.config.ts
// so the OpenTelemetry node-stub aliases don't interfere with tests.
export default defineConfig({
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
  },
})
