// A pretend desktop app for browser tests and the demo: open any mock page with `?shell=1` and `window.koraShell`
// exists, so the "inside the app" behaviour (Browse buttons, notifications) can be tried without the real app.
// Every call is recorded in `window.__koraShellCalls`; `window.__koraShellNext` sets what the next picker answers.
import type { KoraShell } from '@/lib/shell'

interface Call {
  cmd: string
  args: unknown
}

declare global {
  interface Window {
    __koraShellCalls?: Call[]
    __koraShellNext?: string | null
  }
}

export function installMockShell(): void {
  const calls: Call[] = (window.__koraShellCalls = [])
  const rec = (cmd: string, args: unknown) => calls.push({ cmd, args })
  const shell: KoraShell = {
    hello: async () => ({ shell: 'kora-desktop', version: 'mock', os: 'windows', features: { pick_file: true, pick_folder: true, open_external: true, notify: true, elevate: true } }),
    pickFile: async (o) => (rec('pick_file', o), window.__koraShellNext === undefined ? 'C:\\Users\\Sam\\Downloads\\backup.zip' : window.__koraShellNext),
    pickFolder: async (o) => (rec('pick_folder', o), window.__koraShellNext === undefined ? 'C:\\FiveM\\my-server' : window.__koraShellNext),
    openExternal: async (url) => void rec('open_external', url),
    notify: async (o) => void rec('notify', o),
    elevate: async (a) => (rec('elevate', a), { started: true }),
  }
  window.koraShell = shell
}
