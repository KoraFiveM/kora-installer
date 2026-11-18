/// <reference types="vitest/config" />
import { fileURLToPath } from 'node:url'
import { defineConfig, loadEnv } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig(({ command, mode }) => {
  const env = loadEnv(mode, process.cwd(), 'VITE_')
  // The mock backend is compiled in only when VITE_MOCK=1, or in `npm run dev` unless VITE_MOCK=0.
  // A compile-time constant lets the bundler drop the mock (and its fake server names) from normal builds.
  const mock = mode !== 'test' && (env.VITE_MOCK === '1' || (command === 'serve' && env.VITE_MOCK !== '0'))
  return {
    plugins: [react(), tailwindcss()],
    define: { __KORA_MOCK__: JSON.stringify(mock) },
    resolve: { alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) } },
    build: {
      outDir: 'dist',
      assetsDir: 'assets',
      sourcemap: false,
      rolldownOptions: {
        output: {
          // Vendor chunks: libraries change far less often than the app, so after a Kora update the browser keeps
          // the cached copies. Pages and server tabs are split by their dynamic imports (see App.tsx).
          codeSplitting: {
            groups: [
              { name: 'vendor-react', test: /node_modules[\\/](react|react-dom|scheduler)[\\/]/, priority: 30 },
              { name: 'vendor-router', test: /node_modules[\\/](react-router|react-router-dom|@remix-run)[\\/]/, priority: 20 },
              { name: 'vendor-query', test: /node_modules[\\/]@tanstack[\\/]/, priority: 20 },
              // Button's tiny Slot stays with the app: the rest of Radix (dialogs, tabs) is only fetched when a dialog or a server page opens.
              { name: 'vendor-slot', test: /node_modules[\\/]@radix-ui[\\/]react-(slot|compose-refs)[\\/]/, priority: 40 },
              { name: 'vendor-radix', test: /node_modules[\\/](@radix-ui|radix-ui|@floating-ui|aria-hidden|react-remove-scroll[\w-]*|react-style-singleton|use-callback-ref|use-sidecar|get-nonce|tslib)[\\/]/, priority: 20 },
              { name: 'vendor-icons', test: /node_modules[\\/]lucide-react[\\/]/, priority: 20 },
            ],
          },
        },
      },
    },
    server: { proxy: mock ? undefined : { '/api': 'http://127.0.0.1:47120' } },
    test: { environment: 'jsdom', globals: true, setupFiles: ['./src/test/setup.ts'], css: false, exclude: ['e2e/**', 'node_modules/**', 'dist*/**'] },
  }
})
