import { defineConfig } from 'vitest/config'
import path from 'path'

// Client OVERALL coverage gate: whole src (stores + components + utils + api),
// scoped out of the vendored/entry files. The heavy presentational components
// (Dashboard, MediaLightbox, etc.) drag the overall %, so the threshold here is
// deliberately modest vs. the strong stores-only gate (vitest.coverage.gate.ts).
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
      reporter: ['text', 'json-summary'],
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
      thresholds: {
        // Measured 2026-08: lines 43.9, stmts 42.1, funcs 35.7, branches 20.6.
        lines: 41,
        statements: 40,
        functions: 33,
        branches: 18,
      },
    },
  },
})
