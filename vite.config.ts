/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'
import process from 'node:process'
const host = process.env.TAURI_DEV_HOST

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [vue(), tailwindcss()],

  test: {
    environment: 'happy-dom',
    include: ['src/**/*.spec.ts'],
    // Component tests run against the mock IPC backend (spec §15.1: UI
    // tests drive "a mocked IPC backend"; T-141): the same VITE_IPC_MOCK
    // flag dev and e2e use, applied to import.meta.env before any module
    // under test loads. The fixture profile is pinned to `s` too — a
    // developer's VITE_IPC_FIXTURES must not leak set L into every test
    // file's first mockState() call. Real-backend selection stays covered
    // by the pure mode tests in src/ipc/mode.spec.ts.
    env: {
      VITE_IPC_MOCK: '1',
      VITE_IPC_FIXTURES: 's',
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: 'ws',
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ['**/src-tauri/**'],
    },
  },
  build: {
    // The CSP (D-022) sets `img-src 'self'`: images must load from
    // same-origin files, never from `data:` URIs. Vite's default
    // `assetsInlineLimit` (4096 bytes) would inline small SVGs into
    // the JS bundle as data URIs where the CSP blocks them, so every asset
    // is emitted as a real file instead.
    assetsInlineLimit: 0,
  },
}))
