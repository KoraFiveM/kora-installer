// Starts the mock backend and mounts the floating switcher. Imported dynamically from src/main.tsx (mock builds only).
import { createRoot } from 'react-dom/client'
import { installMock } from './index'
import { MockSwitcher } from './Switcher'
import { installMockShell } from './shell'

export function bootMock(): void {
  const scenario = installMock()
  // `?shell=1`: behave as if inside the Kora desktop app (see ./shell.ts).
  if (new URLSearchParams(window.location.search).has('shell')) installMockShell()
  const el = document.createElement('div')
  el.id = 'kora-mock-switcher'
  document.body.appendChild(el)
  createRoot(el).render(<MockSwitcher scenario={scenario} />)
}
