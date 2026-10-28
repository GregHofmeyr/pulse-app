/// <reference types="vitest/config" />
import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vite'

// The app CSP is `font-src 'self'`, which blocks data: URIs, so fonts are never inlined.
export const assetsInlineLimit = (file: string): boolean | undefined =>
  /\.woff2?$/.test(file) ? false : undefined

// Port 1420 is reserved for Pulse (see plan Global Constraints).
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  build: { assetsInlineLimit },
  server: { port: 1420, strictPort: true },
  // Tests: resolve Svelte's client runtime, so $effect and mount() work under vitest (the default is the SSR build).
  resolve: process.env.VITEST ? { conditions: ['browser'] } : undefined,
  test: { setupFiles: ['./src/test-setup.ts'] },
})
