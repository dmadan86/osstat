import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

// Tauri drives the dev server, so the port is fixed and failing loudly is
// better than silently moving to another port the Rust side is not watching.
const DEV_SERVER_PORT = 1420;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  // Prevent Vite from obscuring Rust compiler errors in the terminal.
  clearScreen: false,
  server: {
    port: DEV_SERVER_PORT,
    strictPort: true,
    watch: {
      // The Rust sources are rebuilt by cargo, not by Vite.
      ignored: ['**/src-tauri/**', '**/target/**'],
    },
  },
  // Expose TAURI_* to the client alongside the usual VITE_* prefix.
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: {
    // Match the webview baselines: WebView2 (Chromium), WKWebView, WebKitGTK.
    target: 'esnext',
    sourcemap: true,
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
  },
});
