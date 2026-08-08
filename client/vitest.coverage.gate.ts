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
        // Staged, raised each wave. Measured 2026-08: lines 94.3, stmts 90.3,
        // funcs 89.6, branches 75.8. Gate set ~4-6pp below to buffer test churn.
        lines: 90,
        functions: 85,
        branches: 70,
        statements: 86,
      },
    },
  },
})
