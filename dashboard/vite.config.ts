import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Relative asset URLs (`base: './'`) so the built `index.html` works when
// served from any path the Rust binary mounts it at, not just site root.
// The dev proxy points `/api/*` at `vitals serve`'s default port so `npm
// run dev` can be used against a real running server without CORS games.
export default defineConfig({
  plugins: [react()],
  base: './',
  build: { outDir: 'dist', assetsDir: 'assets' },
  server: { proxy: { '/api': 'http://127.0.0.1:9876' } },
})
