import { afterEach, describe, expect, it, vi } from 'vitest'
import { browseForFile, browseForFolder, elevateInstallService, getShell, inShell, notifyIfAway, type KoraShell } from './shell'

function fake(over: Partial<KoraShell> = {}): KoraShell {
  return {
    hello: vi.fn(async () => ({ shell: 'kora-desktop', version: '1', os: 'windows' as const, features: { pick_file: true, pick_folder: true, open_external: true, notify: true, elevate: true } })),
    pickFile: vi.fn(async () => 'C:\\a.zip'),
    pickFolder: vi.fn(async () => 'C:\\srv'),
    openExternal: vi.fn(async () => undefined),
    notify: vi.fn(async () => undefined),
    elevate: vi.fn(async () => ({ started: true })),
    ...over,
  }
}

afterEach(() => {
  delete window.koraShell
  vi.restoreAllMocks()
})

describe('in a plain browser', () => {
  it('nothing exists and every helper does nothing', async () => {
    expect(getShell()).toBeNull()
    expect(inShell()).toBe(false)
    expect(await browseForFolder('x')).toBeNull()
    expect(await browseForFile('x', ['zip'])).toBeNull()
    expect(await elevateInstallService()).toBe(false)
    expect(() => notifyIfAway({ title: 'x' })).not.toThrow()
  })
})

describe('inside the desktop app', () => {
  it('browsing returns the chosen path and survives a cancel or an error', async () => {
    const s = fake()
    window.koraShell = s
    expect(inShell()).toBe(true)
    expect(await browseForFolder('Choose')).toBe('C:\\srv')
    expect(s.pickFolder).toHaveBeenCalledWith({ title: 'Choose' })
    expect(await browseForFile('Backup', ['zip'])).toBe('C:\\a.zip')
    expect(s.pickFile).toHaveBeenCalledWith({ title: 'Backup', extensions: ['zip'] })
    window.koraShell = fake({ pickFolder: vi.fn(async () => null), pickFile: vi.fn(async () => { throw new Error('no') }) })
    expect(await browseForFolder()).toBeNull()
    expect(await browseForFile('x', [])).toBeNull()
  })

  it('notifies only when the person is not looking at the window', () => {
    const s = fake()
    window.koraShell = s
    vi.spyOn(document, 'hasFocus').mockReturnValue(true)
    notifyIfAway({ title: 'Done' })
    expect(s.notify).not.toHaveBeenCalled()
    vi.spyOn(document, 'hasFocus').mockReturnValue(false)
    notifyIfAway({ title: 'Done', body: 'x', route: '/servers/a' })
    expect(s.notify).toHaveBeenCalledWith({ title: 'Done', body: 'x', route: '/servers/a' })
  })

  it('a refused notification is not an error', () => {
    window.koraShell = fake({ notify: vi.fn(async () => { throw new Error('refused') }) })
    vi.spyOn(document, 'hasFocus').mockReturnValue(false)
    expect(() => notifyIfAway({ title: 'x' })).not.toThrow()
  })

  it('the administrator action runs only when the app says it can', async () => {
    const s = fake()
    window.koraShell = s
    expect(await elevateInstallService()).toBe(true)
    expect(s.elevate).toHaveBeenCalledWith('install_service')
    window.koraShell = fake({ hello: vi.fn(async () => ({ shell: 'k', version: '1', os: 'linux' as const, features: { pick_file: true, pick_folder: true, open_external: true, notify: true, elevate: false } })) })
    expect(await elevateInstallService()).toBe(false)
  })
})
