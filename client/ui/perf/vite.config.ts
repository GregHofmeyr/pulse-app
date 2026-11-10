// Production build of the perf harness (#238): `pnpm exec vite build -c perf/vite.config.ts --outDir <dir>`, served
// with `pnpm exec vite preview -c perf/vite.config.ts --outDir <dir> --port <p>`; open /perf/ (see perf/measure.mjs).
import { fileURLToPath } from 'node:url'
import { defineConfig, mergeConfig } from 'vite'
import base from '../vite.config.ts'

export default mergeConfig(base, defineConfig({
  root: fileURLToPath(new URL('..', import.meta.url)),
  build: { target: 'esnext', rollupOptions: { input: fileURLToPath(new URL('./index.html', import.meta.url)) } },
  server: { port: 14238, strictPort: false },
  preview: { port: 14238, strictPort: false },
}))
