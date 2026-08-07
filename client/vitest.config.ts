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
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json', 'json-summary', 'html'],
      reportsDirectory: 'coverage',
      include: ['src/**/*.{ts,tsx}'],
      exclude: [
        '**/*.test.ts',
        '**/*.test.tsx',
        'src/stores/RootStore.ts',
        'src/stubs/**',
        'src/types/**',
        'src/telemetry.ts',
        'src/main.tsx',
      ],
    },
  },
})
