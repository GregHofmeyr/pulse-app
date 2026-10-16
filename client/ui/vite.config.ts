import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vite'

// Port 1420 is reserved for Pulse (see plan Global Constraints).
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  // Tests: resolve Svelte's client runtime, so $effect and mount() work under vitest (the default is the SSR build).
  resolve: process.env.VITEST ? { conditions: ['browser'] } : undefined,
})
