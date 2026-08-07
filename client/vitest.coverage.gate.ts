import { defineConfig } from 'vitest/config'
import path from 'path'

// Client COVERAGE GATE: scoped to the stores (the unit-testable application
// logic). Thresholds are staged and raised as coverage grows. Run with:
//   npm run coverage:gate
// The broad whole-repo report (including components) is `npm run coverage:full`.
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
      include: ['src/stores/*.ts'],
      exclude: ['**/*.test.ts', 'src/stores/RootStore.ts', 'src/stubs/**'],
      thresholds: {
        // Staged, raised each wave (currently ~45%/33%/41%/43%).
        lines: 40,
        functions: 38,
        branches: 30,
        statements: 40,
      },
    },
  },
})
