import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactElement } from 'react'
import type { BackupInfo, BackupsView, Meta, RestorePreview, ServerDetail } from '@/api/types'
import { jsonResponse } from '@/test/utils'
import { BackupsTab } from './BackupsTab'

const job = vi.hoisted(() => ({ view: { phase: 'idle', events: [] as unknown[], result: null as unknown, error: null as unknown } }))
const IDLE = { phase: 'idle', events: [], result: null, error: null }
vi.mock('@/hooks/useJob', () => ({ useJob: (id: string | null) => (id ? job.view : IDLE) }))

const fetchMock = vi.fn()
beforeEach(() => {
  fetchMock.mockReset()
  job.view = { phase: 'idle', events: [], result: null, error: null }
  vi.stubGlobal('fetch', fetchMock)
})
afterEach(() => vi.unstubAllGlobals())

const server = { name: 'prod', mode: 'production', status: 'running', running: true } as unknown as ServerDetail

const backup = (o: Partial<BackupInfo> = {}): BackupInfo => ({ id: '20260317-041507', file: 'prod-20260317-041507.tar.gz', bytes: 48_000_000, sha256: 'ab', encrypted: false, created_unix: Math.floor(Date.now() / 1000) - 3 * 86400, label: 'manual', kora_version: '0.8.0', server: 'prod', database: 'dumped', by_reference: 38, in_full: 2, stopped: false, too_new: false, ...o })

const view = (o: Partial<BackupsView> = {}): BackupsView => ({
  backups: [],
  settings: { folder: '', retention: { keep_last: 7, keep_daily_days: 0 }, schedule: { enabled: false, cadence: 'daily', time: '03:30' }, resources: 'auto', stop_for_scheduled: false },
  summary: { last_ok_unix: null, text: 'No backup yet', warning: null, scheduled: false },
  folder: '/home/me/.kora/server-backups/prod',
  default_folder: true,
  free_bytes: 50_000_000_000,
  undo: null,
  database: { kind: 'dumped', note: "The server's own database is copied as one consistent snapshot.", tool_missing: false },
  schedule_note: "Scheduled backups run while Kora's background service is running.",
  ...o,
})

const preview = (o: Partial<RestorePreview> = {}): RestorePreview => ({ id: '20260317-041507', created_unix: 1, encrypted: false, needs_passphrase: false, server: null, database: null, resources: [], stopped: false, server_running: true, will_change: ['The server\'s settings'], will_keep: ['secrets.cfg and database.json stay as they are now'], warnings: ['A safety copy of the server as it is now is made first, so you can undo this.'], ...o })

function renderAs(ui: ReactElement, meta?: Partial<Meta>) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  if (meta) qc.setQueryData(['meta'], { version: '0', api: 1, platform: 'linux', ops: [], node: 'n', ...meta })
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>)
}

const answers = (v: BackupsView, extra: (url: string, init?: RequestInit) => Response | undefined = () => undefined) =>
  fetchMock.mockImplementation(async (url: string, init?: RequestInit) => extra(url, init) ?? (String(url).endsWith('/backups') && (init?.method ?? 'GET') === 'GET' ? jsonResponse(200, v) : jsonResponse(200, {})))

describe('BackupsTab', () => {
  it('says plainly what a backup holds and starts with no backup', async () => {
    answers(view())
    renderAs(<BackupsTab server={server} />)
    expect(await screen.findByText('No backup yet', { selector: 'h3, div, p, span' })).toBeInTheDocument()
    expect(screen.getByText(/A backup holds your secrets/)).toBeInTheDocument()
    expect(screen.getByText(/Press "Back up now"/)).toBeInTheDocument()
  })

  it('asks for the password twice, refuses a short or different one, and warns it cannot be recovered', async () => {
    const user = userEvent.setup()
    answers(view())
    renderAs(<BackupsTab server={server} />)
    await screen.findByText(/A backup holds your secrets/)
    await user.click(screen.getByLabelText(/Protect the file with a password/))
    expect(screen.getByText(/cannot be recovered|nobody can recover it/)).toBeInTheDocument()
    const make = screen.getByRole('button', { name: 'Back up now' })
    await user.type(screen.getByLabelText('Password'), 'short')
    expect(await screen.findByText('Use at least 8 characters.')).toBeInTheDocument()
    expect(make).toBeDisabled()
    await user.clear(screen.getByLabelText('Password'))
    await user.type(screen.getByLabelText('Password'), 'a long enough password')
    await user.type(screen.getByLabelText('Password again'), 'a different password')
    expect(await screen.findByText('The two passwords are not the same.')).toBeInTheDocument()
    expect(make).toBeDisabled()
    await user.clear(screen.getByLabelText('Password again'))
    await user.type(screen.getByLabelText('Password again'), 'a long enough password')
    expect(make).toBeEnabled()
  })

  it('sends the options with the backup request', async () => {
    const user = userEvent.setup()
    answers(view(), (_u, init) => (init?.method === 'POST' ? jsonResponse(202, { job: 'job-1' }) : undefined))
    renderAs(<BackupsTab server={server} />)
    await screen.findByText(/A backup holds your secrets/)
    await user.click(screen.getByLabelText(/Stop the server while the backup runs/))
    await user.click(screen.getByLabelText(/Keep every resource in full/))
    await user.click(screen.getByRole('button', { name: 'Back up now' }))
    await waitFor(() => expect(fetchMock.mock.calls.some(([, i]) => i?.method === 'POST')).toBe(true))
    const post = fetchMock.mock.calls.find(([, i]) => i?.method === 'POST')!
    expect(String(post[0])).toContain('/servers/prod/backups')
    expect(JSON.parse(String(post[1].body))).toEqual({ everything: true, stop_server: true, for_move: false })
  })

  it('lists backups with their protection, and warns when the last one is late or failed', async () => {
    answers(view({ backups: [backup(), backup({ id: '2', encrypted: true, label: 'scheduled' })], summary: { last_ok_unix: 1, text: 'Last backup 3 days ago', warning: 'The last backup is older than a day, but backups are meant to run every day.', scheduled: true } }))
    renderAs(<BackupsTab server={server} />)
    expect(await screen.findByText(/older than a day/)).toBeInTheDocument()
    expect(screen.getByText('Password protected')).toBeInTheDocument()
    expect(screen.getByText('Not password protected')).toBeInTheDocument()
    expect(screen.getByText('Scheduled')).toBeInTheDocument()
  })

  it('restores a password protected backup: password first, then what will change, then the restore', async () => {
    const user = userEvent.setup()
    const enc = backup({ encrypted: true })
    answers(view({ backups: [enc] }), (u, init) => {
      if (String(u).endsWith('/preview')) {
        const pw = JSON.parse(String(init?.body)).passphrase
        return pw === 'right password' ? jsonResponse(200, preview({ encrypted: true })) : jsonResponse(422, { error: { code: 'backup.wrong_passphrase', message: 'That password does not open this backup.', actions: [] } })
      }
      if (String(u).endsWith('/restore')) return jsonResponse(202, { job: 'job-2' })
      return undefined
    })
    renderAs(<BackupsTab server={server} />)
    await user.click(await screen.findByRole('button', { name: 'Restore…' }))
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText("This backup's password"), 'wrong')
    await user.click(within(dialog).getByRole('button', { name: 'Open the backup' }))
    expect(await within(dialog).findByText(/does not open this backup/)).toBeInTheDocument()
    await user.clear(within(dialog).getByLabelText("This backup's password"))
    await user.type(within(dialog).getByLabelText("This backup's password"), 'right password')
    await user.click(within(dialog).getByRole('button', { name: 'Open the backup' }))
    expect(await within(dialog).findByText('This will:')).toBeInTheDocument()
    expect(within(dialog).getByText(/secrets.cfg and database.json stay as they are now/)).toBeInTheDocument()
    expect(within(dialog).getByText(/safety copy of the server as it is now/)).toBeInTheDocument()
    await user.click(within(dialog).getByRole('button', { name: 'Restore now' }))
    await waitFor(() => expect(fetchMock.mock.calls.some(([u]) => String(u).endsWith('/restore'))).toBe(true))
    const call = fetchMock.mock.calls.find(([u]) => String(u).endsWith('/restore'))!
    expect(JSON.parse(String(call[1].body))).toEqual({ passphrase: 'right password' })
  })

  it('shows the result of a restore with what did not work', async () => {
    const user = userEvent.setup()
    job.view = { phase: 'done', events: [], error: null, result: { safety_backup: 's', database_restored: true, files_restored: 9, resources_restored: ['core is back at version abcdef12'], problems: ['kora_hud: could not fetch from https://github.com/x (offline). It was left as it is.'], notes: [], restarted: true } }
    answers(view({ backups: [backup()] }), (u) => (String(u).endsWith('/preview') ? jsonResponse(200, preview()) : String(u).endsWith('/restore') ? jsonResponse(202, { job: 'job-9' }) : undefined))
    renderAs(<BackupsTab server={server} />)
    await user.click(await screen.findByRole('button', { name: 'Restore…' }))
    const dialog = await screen.findByRole('dialog')
    await user.click(await within(dialog).findByRole('button', { name: 'Restore now' }))
    expect(await within(dialog).findByText('Restored, with some things to look at')).toBeInTheDocument()
    expect(within(dialog).getByText(/could not fetch from/)).toBeInTheDocument()
    expect(within(dialog).getByText('core is back at version abcdef12')).toBeInTheDocument()
  })

  it('offers "Undo this restore" when a safety copy exists', async () => {
    const user = userEvent.setup()
    answers(view({ backups: [backup({ id: 's1', label: 'before_restore' })], undo: { id: 's1', created_unix: 1, encrypted: false } }), (u) => (String(u).endsWith('/backups/undo') ? jsonResponse(202, { job: 'job-3' }) : undefined))
    renderAs(<BackupsTab server={server} />)
    await user.click(await screen.findByRole('button', { name: /Undo this restore/ }))
    await user.click(screen.getByRole('button', { name: 'Put the server back as it was' }))
    await waitFor(() => expect(fetchMock.mock.calls.some(([u]) => String(u).endsWith('/backups/undo'))).toBe(true))
  })

  it('an operator may back up but not restore, delete or change settings, and is told why', async () => {
    answers(view({ backups: [backup()] }))
    renderAs(<BackupsTab server={server} />, { identity: { name: 'sam', provider: 'token', role: 'operator' }, remote: true })
    expect(await screen.findByRole('button', { name: 'Restore…' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Back up now' })).toBeEnabled()
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled()
    expect(screen.getAllByText(/cannot do this/).length).toBeGreaterThan(0)
  })

  it('a viewer sees why there is nothing to see', async () => {
    renderAs(<BackupsTab server={server} />, { identity: { name: 'v', provider: 'token', role: 'viewer' }, remote: true })
    expect(await screen.findByText(/cannot do this/)).toBeInTheDocument()
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it('tells when the database tools are missing', async () => {
    answers(view({ database: { kind: 'dumped', note: 'x', tool_missing: true } }))
    renderAs(<BackupsTab server={server} />)
    expect(await screen.findByText('The database tools are missing')).toBeInTheDocument()
  })

  it('saves a schedule with the time and weekday', async () => {
    const user = userEvent.setup()
    answers(view(), (_u, init) => (init?.method === 'PUT' ? jsonResponse(200, JSON.parse(String(init.body))) : undefined))
    renderAs(<BackupsTab server={server} />)
    await screen.findByText(/Scheduled backups/)
    await user.click(screen.getByLabelText(/Once a week/))
    await user.clear(screen.getByLabelText('Time (24 hours)'))
    await user.type(screen.getByLabelText('Time (24 hours)'), '04:15')
    await user.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(fetchMock.mock.calls.some(([, i]) => i?.method === 'PUT')).toBe(true))
    const put = JSON.parse(String(fetchMock.mock.calls.find(([, i]) => i?.method === 'PUT')![1].body))
    expect(put.schedule).toEqual({ enabled: true, cadence: 'weekly', time: '04:15', weekday: 6 })
  })
})
