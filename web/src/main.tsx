import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './fonts.css'
import './index.css'
import { App } from './App'
import { bootstrapSession } from './lib/session'

async function start() {
  // __KORA_MOCK__ is a compile-time constant (see vite.config.ts): when false, the mock is not in the bundle.
  if (__KORA_MOCK__) {
    const { bootMock } = await import('./api/mock/boot')
    bootMock()
  }
  const boot = await bootstrapSession()
  createRoot(document.getElementById('root')!).render(
    <StrictMode>
      <App boot={boot} />
    </StrictMode>,
  )
}

void start()
