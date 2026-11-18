// Inside the desktop app the system dialogs fill the boxes for "choose a backup file" and "choose a backup folder".
import { afterEach, describe, expect, it, vi } from 'vitest'
import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { jsonResponse, renderApp } from '@/test/utils'
import type { KoraShell } from '@/lib/shell'
import { BackupsTab } from '@/pages/server/BackupsTab'
import ImportPage from './ImportPage'

afterEach(() => {
  delete window.koraShell
  vi.unstubAllGlobals()
})

const shell = (file: string | null, folder: string | null) => ({ hello: vi.fn(), pickFile: vi.fn(async () => file), pickFolder: vi.fn(async () => folder), openExternal: vi.fn(), notify: vi.fn(), elevate: vi.fn() }) as unknown as KoraShell

describe('desktop app dialogs', () => {
  it('Browse for the backup file fills the path and checks the file', async () => {
    const s = shell('D:\\Moves\\old.tar.gz.age', null)
    window.koraShell = s
    const fetchMock = vi.fn(async () => jsonResponse(200, { needs_passphrase: true, file_name: 'old.tar.gz.age', encrypted: true, created_unix: 1, server: null, database: null, by_reference: 0, in_full: 0, suggested: null, clashes: [], will_create: [], wont_carry: [], warnings: [], saved_keys: [], licence_note: '', fork: null, db_login: null, servers: [] }))
    vi.stubGlobal('fetch', fetchMock)
    renderApp(<ImportPage />, '/import')
    await userEvent.click(await screen.findByRole('button', { name: 'Browse...' }))
    expect(s.pickFile).toHaveBeenCalledWith({ title: 'Choose the backup file', extensions: ['age', 'gz'] })
    await waitFor(() => expect(screen.getByLabelText('Its password')).toBeInTheDocument())
    const call = (fetchMock.mock.calls as unknown as [string, RequestInit][]).find(([u]) => String(u).endsWith('/backups/import/inspect'))!
    expect(JSON.parse(String(call[1].body)).source).toEqual({ path: 'D:\\Moves\\old.tar.gz.age' })
  })

  it('Browse for the backups folder fills the folder box, and a cancelled dialog changes nothing', async () => {
    window.koraShell = shell(null, 'E:\\Backups')
    const view = { backups: [], settings: { folder: '', retention: { keep_last: 7, keep_daily_days: 0 }, schedule: { enabled: true, cadence: 'daily', time: '03:30' }, resources: 'auto', stop_for_scheduled: false }, summary: { last_ok_unix: null, text: 'No backup yet', warning: null, scheduled: true }, folder: '/x', default_folder: true, free_bytes: null, undo: null, database: { kind: 'none', note: '', tool_missing: false }, schedule_note: '' }
    vi.stubGlobal('fetch', vi.fn(async () => jsonResponse(200, view)))
    renderApp(<BackupsTab server={{ name: 'prod', status: 'stopped', running: false } as never} />, '/')
    await userEvent.click(await screen.findByRole('button', { name: 'Browse...' }))
    await waitFor(() => expect(screen.getByLabelText(/Use another folder/)).toHaveValue('E:\\Backups'))
    window.koraShell = shell(null, null)
  })
})
