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
    setupFiles: ['./vitest.setup.ts'],
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
        lines: 45,
        statements: 43,
        functions: 38,
        branches: 23,
      },
    },
  },
})
