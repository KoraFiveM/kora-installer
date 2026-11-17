// The Kora desktop app (tray icon and window) adds `window.koraShell` before the page runs. In a plain browser it does
// not exist, and everything here quietly does nothing: the page behaves exactly as it always did.
//
// The app only offers the small list below (see crates/kora-shell/src/bridge.rs): pick a file or folder with the
// system dialog, open a web link in the real browser, show a desktop notification, and one fixed administrator action.

export interface ShellHello {
  shell: string
  version: string
  os: 'windows' | 'linux'
  features: { pick_file: boolean; pick_folder: boolean; open_external: boolean; notify: boolean; elevate: boolean }
}

export interface KoraShell {
  hello(): Promise<ShellHello>
  /** The chosen path, or null when the person cancelled. */
  pickFile(o?: { title?: string; extensions?: string[] }): Promise<string | null>
  pickFolder(o?: { title?: string }): Promise<string | null>
  openExternal(url: string): Promise<void>
  notify(o: { title: string; body?: string; route?: string; urgent?: boolean }): Promise<void>
  elevate(action: 'install_service'): Promise<{ started: boolean }>
}

declare global {
  interface Window {
    koraShell?: KoraShell
  }
}

/** The desktop app's bridge, or null in a normal browser. */
export function getShell(): KoraShell | null {
  return typeof window !== 'undefined' && window.koraShell ? window.koraShell : null
}

export function inShell(): boolean {
  return getShell() !== null
}

/** "Browse..." next to a folder box: the system folder dialog. null = no desktop app here, or the person cancelled. */
export async function browseForFolder(title?: string): Promise<string | null> {
  const s = getShell()
  if (!s) return null
  try {
    return await s.pickFolder({ title })
  } catch {
    return null
  }
}

export async function browseForFile(title: string, extensions: string[]): Promise<string | null> {
  const s = getShell()
  if (!s) return null
  try {
    return await s.pickFile({ title, extensions })
  } catch {
    return null
  }
}

/**
 * A desktop notification for something that finished or failed, only when the person is not looking at the window
 * (hidden to the tray, minimised or behind another program). The notification opens `route` when clicked.
 */
export function notifyIfAway(o: { title: string; body?: string; route?: string; urgent?: boolean }): void {
  const s = getShell()
  if (!s || (typeof document !== 'undefined' && document.hasFocus() && !document.hidden)) return
  void s.notify(o).catch(() => undefined)
}

/**
 * Run "Install Kora's administrator service" from the app, so Windows' permission prompt opens in front of the app
 * instead of as a flashing icon in the taskbar. false = not in the app (the caller does what it did before).
 */
export async function elevateInstallService(): Promise<boolean> {
  const s = getShell()
  if (!s || !(await s.hello().catch(() => null))?.features.elevate) return false
  try {
    return (await s.elevate('install_service')).started
  } catch {
    return false
  }
}
