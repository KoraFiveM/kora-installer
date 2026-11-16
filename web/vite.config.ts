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
    build: { outDir: 'dist', assetsDir: 'assets', sourcemap: false },
    server: { proxy: mock ? undefined : { '/api': 'http://127.0.0.1:47120' } },
    test: { environment: 'jsdom', globals: true, setupFiles: ['./src/test/setup.ts'], css: false },
  }
})
