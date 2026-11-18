import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './fonts.css'
import './index.css'
import { App } from './App'
import { bootstrapSession } from './lib/session'
import { boot as bootLanguage } from './i18n'

async function start() {
  // __KORA_MOCK__ is a compile-time constant (see vite.config.ts): when false, the mock is not in the bundle.
  if (__KORA_MOCK__) {
    const { bootMock } = await import('./api/mock/boot')
    bootMock()
  }
  // The language (remembered, else from the browser's list) is loaded before the first screen is drawn.
  await bootLanguage()
  const boot = await bootstrapSession()
  createRoot(document.getElementById('root')!).render(
    <StrictMode>
      <App boot={boot} />
    </StrictMode>,
  )
}

void start()
