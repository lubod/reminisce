import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'path'

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
      path: "path-browserify",
      // Stub out Node.js-specific OpenTelemetry files
      "@opentelemetry/instrumentation/build/esm/platform/node": path.resolve(__dirname, "./src/stubs/otel-node-stub.ts"),
      "@opentelemetry/instrumentation/build/esm/instrumentationNodeModuleFile": path.resolve(__dirname, "./src/stubs/otel-node-stub.ts"),
    },
    conditions: ['browser', 'module', 'import', 'default'],
  },
  optimizeDeps: {
    exclude: [
      '@opentelemetry/instrumentation/platform/node',
      '@opentelemetry/instrumentation-user-interaction',
    ],
    esbuildOptions: {
      define: {
        global: 'globalThis',
      },
      alias: {
        path: 'path-browserify',
      },
    },
  },
  server: {
    host: '0.0.0.0',  // Listen on all network interfaces (accessible from other PCs)
    port: 5173,
    proxy: {
      '/api': {
        target: 'http://localhost:8080',  // Local dev: direct to reminisce (no nginx)
        changeOrigin: true,
        secure: false,
        rewrite: (path) => path.replace(/^\/api/, ''),  // Strip /api prefix
      },
    },
  },
})
